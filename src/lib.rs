//! RS-SmartTCP: IOWN/APN(光電融合ネットワーク)× Smart-TCP(AI生成通信
//! プロトコル)の良いとこ取りハイブリッド適応制御。
//!
//! ## 発想の出典(日英Web検索で裏取り済み、2026-07-23)
//!
//! - **IOWN/APN**: NTTのオールフォトニクス・ネットワークは、日本-台湾間
//!   3,000kmで約17ms・**ジッター無し**という特性を実証済み
//!   ([digitimes: NTT IOWN 2026](https://www.digitimes.com/news/a20251007PD227/ntt-iown-infrastructure-launch-2026.html))。
//!   これは物理telecom基盤であり、このクレートが「実装」できる対象では
//!   ない——ただし、そのような超低遅延・ジッター無し回線が使える場面では、
//!   ソフトウェア層が余計なバッファリング・保守的すぎるリトライ間隔で
//!   その利点を無駄にしないことには意味がある。
//! - **Smart-TCP**(arXiv 2512.00491、2026年7月更新、"Agentic AI-based
//!   Autonomous and Adaptive TCP Protocol"): TCP制御ロジックを
//!   「fast/slowの2モデルによる判断プロセス」として構成する、という
//!   AI生成プロトコルの設計思想。
//!
//! ## RTT/ジッター推定アルゴリズムの再設計(2026-07-23、実装方式を
//! 再検証)
//!
//! 当初は固定ウィンドウ+標準偏差でジッターを推定していたが、ユーザー
//! 指示により実装方式そのものを再検証した結果、**TCP(RFC 6298)と
//! QUIC(RFC 9002)が実際にどちらも採用している SRTT/RTTVAR
//! (Jacobson/Karels の指数移動平均アルゴリズム)** が業界標準の実装
//! 方式だと確認できた
//! ([RFC 6298](https://www.rfc-editor.org/rfc/rfc6298.html)、
//! [RFC 9002](https://www.rfc-editor.org/rfc/rfc9002.xml))。
//! 固定ウィンドウ方式(全サンプルをメモリに保持し、クエリのたびに
//! 平均・分散を計算し直す)より、SRTT/RTTVARはサンプル1件ごとに
//! **O(1)の更新のみ**で済み、かつTCP/QUIC双方の輻輳制御と全く同じ
//! 枯れたアルゴリズムであるため、この書き換えを採用した(このエコ
//! システムが既にQUIC(`quic_channel`)を使っていることとも整合する)。
//!
//! - `SRTT`(smoothed RTT): `SRTT = (1-α)·SRTT + α·R'`、α=1/8
//! - `RTTVAR`(RTT variation): `RTTVAR = (1-β)·RTTVAR + β·|SRTT-R'|`、β=1/4
//! - 初回サンプルのみ特別扱い: `SRTT=R`, `RTTVAR=R/2`(RFC 6298 2.2節)
//!
//! ## 正直な開示・命名の経緯
//!
//! **本クレートは、上記arXiv論文の"Smart-TCP"プロトコルそのものの実装
//! ではない。** 訓練済み機械学習モデルは使わず、「fast/slowモデル」
//! という設計思想を、**RFC 6298/9002と同じSRTT/RTTVAR推定に基づく
//! 決定論的な2値判定**として実装したものであり、`RS-SmartTCP`という
//! 名前は「Smart-TCPに着想を得た、このエコシステム(`aon-co-jp`)独自の
//! 実装」であることを示す(既存の`RS-`接頭辞の命名規則に準拠、
//! `RS-Git`/`RS-Guard`/`RS-JSON`等と同じ扱い)。論文の同名プロトコルと
//! 混同しないこと。
//!
//! [`NetworkQualityMonitor`]がSRTT・RTTVARを追跡し、両方が閾値未満
//! なら「photonic-class」(IOWN/APNのような光ネットワーク級)、そうで
//! なければ「standard-class」(通常のインターネット経路)と判定する。
//! [`AdaptivePolicy`]はこの判定に応じて、リトライ間隔等の呼び出し側が
//! 握っている決定を2段階(fast/slow)で切り替える。
//!
//! ## このエコシステムでの利用箇所
//!
//! [`open-web-server-wire`](https://github.com/aon-co-jp/open-web-server)
//! から、path依存として利用される(`Rust-JSON`が`aruaru-db`等から
//! 利用されるのと同じ「独立リポジトリとして切り出し、必要な場所から
//! path依存する」パターン)。

use std::sync::Mutex;
use std::time::Duration;

pub mod bandwidth_policy;
pub mod download_protection;
pub mod maintenance;
pub mod multi_path;
pub mod multi_wan;
pub mod network_interfaces;
pub mod link_diagnostics;
pub mod path_optimizer;
pub mod raid_bridge;
pub mod redundant_transmission;
pub mod router_features;
pub mod secure_channel;
pub mod tls_inspection;
pub mod transaction_log;
pub mod usb_protection;
pub mod wan_config;
pub mod wifi_roadmap;

pub use bandwidth_policy::{BandwidthPolicy, TrafficPurpose};
pub use multi_path::MultiPathManager;
pub use network_interfaces::{InterfaceKind, NetworkInterfaceReport};

/// TCP(RFC 6298)と同じ重み。QUIC(RFC 9002)も同じαを採用している。
const ALPHA: f64 = 1.0 / 8.0;
/// TCP(RFC 6298)と同じ重み。
const BETA: f64 = 1.0 / 4.0;

#[derive(Debug, Clone, Copy)]
struct SrttState {
    srtt_ms: f64,
    rttvar_ms: f64,
}

/// RTTサンプルからSRTT/RTTVAR(RFC 6298の`Computing TCP's Retransmission
/// Timer`と同一のJacobson/Karels EWMAアルゴリズム)を追跡し、ネットワーク
/// 品質を分類する。
pub struct NetworkQualityMonitor {
    state: Mutex<Option<SrttState>>,
    /// この値未満のRTTVAR(ミリ秒)を「photonic-class」とみなす閾値。
    /// 既定5msはIOWN/APNの実証値(ジッター無し)に対し十分余裕を持たせた
    /// 保守的な値。
    rttvar_threshold_ms: f64,
    /// この値未満のSRTT(ミリ秒)も「photonic-class」の条件に含める
    /// (RTTVARが低くてもRTT自体が高ければ光ネットワーク級とは呼べない)。
    srtt_threshold_ms: f64,
}

impl NetworkQualityMonitor {
    pub fn new() -> Self {
        Self::with_thresholds(5.0, 50.0)
    }

    pub fn with_thresholds(rttvar_threshold_ms: f64, srtt_threshold_ms: f64) -> Self {
        Self { state: Mutex::new(None), rttvar_threshold_ms, srtt_threshold_ms }
    }

    /// 1回のRTT実測値を記録する(呼び出し側が実際の往復時間を計測して
    /// 渡す)。RFC 6298 2.2節の通り、初回サンプルは`SRTT=R, RTTVAR=R/2`
    /// で初期化し、以後はEWMAで更新する。
    pub fn record_rtt(&self, rtt: Duration) {
        let r = rtt.as_secs_f64() * 1000.0;
        let mut state = self.state.lock().unwrap();
        *state = Some(match *state {
            None => SrttState { srtt_ms: r, rttvar_ms: r / 2.0 },
            Some(prev) => {
                let rttvar_ms = (1.0 - BETA) * prev.rttvar_ms + BETA * (prev.srtt_ms - r).abs();
                let srtt_ms = (1.0 - ALPHA) * prev.srtt_ms + ALPHA * r;
                SrttState { srtt_ms, rttvar_ms }
            }
        });
    }

    /// 現在のSRTT(平滑化RTT、ミリ秒)。サンプルが無ければNone。
    pub fn smoothed_rtt_ms(&self) -> Option<f64> {
        self.state.lock().unwrap().map(|s| s.srtt_ms)
    }

    /// 現在のRTTVAR(RTT変動、ミリ秒)。サンプルが無ければNone。
    pub fn rttvar_ms(&self) -> Option<f64> {
        self.state.lock().unwrap().map(|s| s.rttvar_ms)
    }

    /// 現在の観測が「photonic-class」(IOWN/APNのような低遅延・低ジッター
    /// 回線)と判定できるか。サンプルが無ければ判定を保留し、安全側
    /// (standard-class)として扱う——未知の回線をいきなり積極的な設定で
    /// 扱わない、という慎重さ。
    pub fn is_photonic_class(&self) -> bool {
        match *self.state.lock().unwrap() {
            Some(s) => s.srtt_ms < self.srtt_threshold_ms && s.rttvar_ms < self.rttvar_threshold_ms,
            None => false,
        }
    }
}

impl Default for NetworkQualityMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Smart-TCPの「fast/slowモデル」設計に倣った、2段階の適応方針。
/// `NetworkQualityMonitor`の判定結果に応じて、呼び出し側が握っている
/// パラメータ(リトライ間隔・UDP即時通知の送出頻度等)を切り替える。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdaptiveMode {
    /// photonic-class回線向け: 積極的な設定(短いリトライ間隔、UDP即時
    /// 通知を間引かず毎回送出)。IOWN/APNのような低遅延・ジッター無し
    /// 回線の利点を、ソフトウェア層の保守的な余裕(マージン)で無駄に
    /// しないための設定。
    Fast,
    /// standard-class回線向け(既定・安全側): 従来通りの保守的な設定。
    Slow,
}

pub struct AdaptivePolicy {
    monitor: NetworkQualityMonitor,
}

impl AdaptivePolicy {
    pub fn new(monitor: NetworkQualityMonitor) -> Self {
        Self { monitor }
    }

    pub fn monitor(&self) -> &NetworkQualityMonitor {
        &self.monitor
    }

    /// 現在のネットワーク品質観測に基づくモード。
    pub fn mode(&self) -> AdaptiveMode {
        if self.monitor.is_photonic_class() {
            AdaptiveMode::Fast
        } else {
            AdaptiveMode::Slow
        }
    }

    /// モードに応じたリトライ待機時間(`open-web-server-ledger::Ledger`の
    /// `retry_backoff`相当の用途)。Fastモードでは光ネットワーク級の
    /// 低遅延・低ジッターを前提に、待機を大きく切り詰められる。
    pub fn retry_backoff(&self) -> Duration {
        match self.mode() {
            AdaptiveMode::Fast => Duration::from_millis(5),
            AdaptiveMode::Slow => Duration::from_millis(200),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_latency_stable_samples_classify_as_photonic_class() {
        let monitor = NetworkQualityMonitor::new();
        // IOWN/APNの実証値(約17ms、ジッター無し)を模した安定したRTT列。
        for _ in 0..10 {
            monitor.record_rtt(Duration::from_millis(17));
        }
        assert!(monitor.is_photonic_class());
        assert!(monitor.smoothed_rtt_ms().unwrap() < 20.0);
        assert!(monitor.rttvar_ms().unwrap() < 1.0);
    }

    #[test]
    fn high_variance_samples_classify_as_standard_class_even_with_low_mean_rtt() {
        let monitor = NetworkQualityMonitor::new();
        // 平均は低いが、ばらつきが大きい(典型的なベストエフォート
        // インターネット経路)ケース。
        for ms in [5u64, 80, 3, 95, 10, 70, 2, 60, 5, 90] {
            monitor.record_rtt(Duration::from_millis(ms));
        }
        assert!(!monitor.is_photonic_class());
    }

    #[test]
    fn no_samples_conservatively_default_to_standard_class() {
        let monitor = NetworkQualityMonitor::new();
        assert!(!monitor.is_photonic_class(), "no data yet must not be treated as the fast/optimistic case");
    }

    /// RFC 6298 2.2節の初回サンプル特別扱い(`SRTT=R, RTTVAR=R/2`)が
    /// 正しく適用されることの実証。
    #[test]
    fn first_sample_initializes_srtt_and_rttvar_per_rfc6298() {
        let monitor = NetworkQualityMonitor::new();
        monitor.record_rtt(Duration::from_millis(100));
        assert_eq!(monitor.smoothed_rtt_ms(), Some(100.0));
        assert_eq!(monitor.rttvar_ms(), Some(50.0));
    }

    #[test]
    fn adaptive_policy_switches_retry_backoff_between_fast_and_slow_modes() {
        let policy = AdaptivePolicy::new(NetworkQualityMonitor::new());
        assert_eq!(policy.mode(), AdaptiveMode::Slow, "no observations yet -> conservative default");
        let slow_backoff = policy.retry_backoff();

        for _ in 0..10 {
            policy.monitor().record_rtt(Duration::from_millis(17));
        }
        assert_eq!(policy.mode(), AdaptiveMode::Fast);
        let fast_backoff = policy.retry_backoff();

        assert!(fast_backoff < slow_backoff, "photonic-class network must unlock a shorter retry backoff");
    }

    /// 高ジッターな観測が後から混ざると、fastからslowへ正しく降格する
    /// (一度「光ネットワーク級」と判定してもそれに固執しない適応性、
    /// EWMAなので急激な変化の反映には数サンプル要することも許容する)。
    #[test]
    fn policy_downgrades_from_fast_to_slow_when_variance_increases() {
        let policy = AdaptivePolicy::new(NetworkQualityMonitor::new());
        for _ in 0..10 {
            policy.monitor().record_rtt(Duration::from_millis(17));
        }
        assert_eq!(policy.mode(), AdaptiveMode::Fast);

        for _ in 0..5 {
            for ms in [5u64, 90, 2, 100, 8, 85] {
                policy.monitor().record_rtt(Duration::from_millis(ms));
            }
        }
        assert_eq!(policy.mode(), AdaptiveMode::Slow, "must react to the network getting worse, not stay stuck in Fast");
    }
}
