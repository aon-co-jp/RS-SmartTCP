//! 通信品質の問題箇所特定+GUI向け診断表示(2026-08-11新設、ユーザー指示
//! 「通信品質に何が原因で問題があるか特定…配線が断線している/断線
//! しかかっている/通信が不安定ならそれを明白にGUIで表示…可能なら自動で
//! 改善する機能」への対応)。
//!
//! ## 正直な開示(最重要・スコープの明確化)
//!
//! - **本モジュールは実際のケーブル断線を検知するハードウェアセンサーを
//!   持たない**——`network_interfaces::detect()`が報告する「OS上の
//!   接続状態(リンクアップ/ダウン)」と、[`crate::NetworkQualityMonitor`]
//!   が実測するRTT/RTTVARから、間接的に「断線している」「断線しかかって
//!   いる(不安定)」を推測するのみ。実際に物理ケーブルを目視点検する
//!   必要性を代替するものではない。
//! - **「自動で改善する」の実体**: 物理的な配線を自動で直すことは
//!   当然できない——ここでの「自動改善」は、[`crate::multi_path::
//!   MultiPathManager::best_path`]が既に行っている「登録済み経路の中で
//!   最もRTTが低い経路へ自動的に traffic を寄せる」という既存の仕組みを
//!   指す。本モジュールは、どの経路が問題を抱えているために除外
//!   されているのか(または除外されるべきなのか)を、ユーザーに分かる
//!   形で明示する診断レイヤーを追加するもの。
//! - 閾値(RTTVAR 30ms/50ms等)は、[`crate::NetworkQualityMonitor`]が
//!   既に採用しているphotonic-class判定の閾値(5ms/50ms)を土台に、
//!   「不安定」を検知するための保守的な目安として設定した経験則であり、
//!   厳密な学術的根拠に基づく値ではない(誇張しない)。

use crate::network_interfaces::NetworkInterfaceReport;
use crate::NetworkQualityMonitor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkHealth {
    /// OS上で接続が確認できない(ケーブル/アダプタの物理的な断線・
    /// 無効化の可能性が高い)。
    Disconnected,
    /// 接続はあるがRTTVAR(ジッター)が大きく振れている——「断線しかかって
    /// いる」典型的な兆候(コネクタの接触不良・ケーブル損傷等でよく
    /// 見られるパターン)。
    Unstable,
    /// 接続はあり安定しているが、RTT自体が高い(遠い/輻輳している経路)。
    Degraded,
    /// RTT実測がまだ無い(判定保留)。
    Unknown,
    Healthy,
}

impl LinkHealth {
    pub fn label_ja(&self) -> &'static str {
        match self {
            LinkHealth::Disconnected => "🔴 未接続(断線の可能性)",
            LinkHealth::Unstable => "🟠 不安定(断線しかかっている可能性)",
            LinkHealth::Degraded => "🟡 低速(RTTが高い)",
            LinkHealth::Unknown => "⚪ 測定待ち",
            LinkHealth::Healthy => "🟢 正常",
        }
    }

    pub fn label_en(&self) -> &'static str {
        match self {
            LinkHealth::Disconnected => "Disconnected (possible cable fault)",
            LinkHealth::Unstable => "Unstable (possible failing connection)",
            LinkHealth::Degraded => "Degraded (high RTT)",
            LinkHealth::Unknown => "Pending measurement",
            LinkHealth::Healthy => "Healthy",
        }
    }
}

/// RTTVARがこの値(ミリ秒)を超えたら「不安定」とみなす。
pub const UNSTABLE_RTTVAR_THRESHOLD_MS: f64 = 30.0;
/// SRTTがこの値(ミリ秒)を超えたら(かつ不安定でなければ)「低速」とみなす。
pub const DEGRADED_RTT_THRESHOLD_MS: f64 = 150.0;

pub struct LinkDiagnosis {
    pub name: String,
    pub connected: bool,
    pub smoothed_rtt_ms: Option<f64>,
    pub rttvar_ms: Option<f64>,
    pub health: LinkHealth,
    pub reason_ja: String,
    pub reason_en: String,
}

/// 接続状態+RTT/RTTVAR実測値から1経路分の診断を行う。
pub fn diagnose(name: &str, connected: bool, monitor: &NetworkQualityMonitor) -> LinkDiagnosis {
    diagnose_with_values(name, connected, monitor.smoothed_rtt_ms(), monitor.rttvar_ms())
}

/// [`diagnose`]の中身(接続状態+RTT/RTTVARの生の値から判定する部分)。
/// `MultiPathManager`側は`PathEntry`(モニタ本体)を非公開のまま保持して
/// いるため、スナップショット値渡しでこの判定ロジックを再利用できるよう
/// 分離した。
fn diagnose_with_values(name: &str, connected: bool, smoothed_rtt_ms: Option<f64>, rttvar_ms: Option<f64>) -> LinkDiagnosis {
    let (health, reason_ja, reason_en) = if !connected {
        (
            LinkHealth::Disconnected,
            "OS上で接続が確認できません。ケーブル・アダプタの物理的な断線や無効化の可能性があります。".to_string(),
            "Not connected at the OS level. The cable/adapter may be physically disconnected or disabled.".to_string(),
        )
    } else {
        match (smoothed_rtt_ms, rttvar_ms) {
            (Some(rtt), Some(rttvar)) if rttvar >= UNSTABLE_RTTVAR_THRESHOLD_MS => (
                LinkHealth::Unstable,
                format!("応答時間のばらつき(RTTVAR)が{rttvar:.1}msと大きく、接続が不安定です。コネクタの接触不良や断線しかかっているケーブルでよく見られる兆候です。"),
                format!("Response time jitter (RTTVAR) is high at {rttvar:.1}ms, indicating an unstable connection — a common sign of a failing connector or cable (SRTT={rtt:.1}ms)."),
            ),
            (Some(rtt), _) if rtt >= DEGRADED_RTT_THRESHOLD_MS => (
                LinkHealth::Degraded,
                format!("応答時間(RTT)が{rtt:.1}msと高めです。遠い経路または輻輳している可能性があります。"),
                format!("Round-trip time is elevated at {rtt:.1}ms — the path may be distant or congested."),
            ),
            (Some(_), Some(_)) => (LinkHealth::Healthy, "正常です。".to_string(), "Healthy.".to_string()),
            _ => (LinkHealth::Unknown, "まだRTTの実測値がありません。".to_string(), "No RTT samples recorded yet.".to_string()),
        }
    };

    LinkDiagnosis { name: name.to_string(), connected, smoothed_rtt_ms, rttvar_ms, health, reason_ja, reason_en }
}

/// [`crate::network_interfaces::detect`]の結果と、`MultiPathManager`側から
/// 得た(経路名, 接続状態, SRTT, RTTVAR)のスナップショットを突き合わせ、
/// 経路ごとの診断一覧を組み立てる(`MultiPathManager::diagnose_paths`が
/// この関数を呼ぶ、`PathEntry`が非公開のため実測値はスナップショットとして
/// 受け渡す設計)。未接続のインターフェースは、経路として未登録でも
/// 診断対象に含める——「断線していることに気づけない」ことを避けるため。
pub fn diagnose_all_from_snapshots(
    report: &NetworkInterfaceReport,
    snapshots: &[(String, bool, Option<f64>, Option<f64>)],
) -> Vec<LinkDiagnosis> {
    let mut results = Vec::new();
    for (name, connected, smoothed_rtt_ms, rttvar_ms) in snapshots {
        results.push(diagnose_with_values(name, *connected, *smoothed_rtt_ms, *rttvar_ms));
    }
    for iface in &report.interfaces {
        if !iface.connected && !snapshots.iter().any(|(name, _, _, _)| name == &iface.name) {
            results.push(diagnose_with_values(&iface.name, false, None, None));
        }
    }
    results
}

/// 「自動改善」の実体: 診断結果から、実際に使うべき(=最も健全な)経路名を
/// 選ぶ。`Healthy`を優先し、無ければ`Degraded`、それも無ければ`None`
/// (呼び出し側は`MultiPathManager::best_path`の既定挙動に委ねる)。
/// `Disconnected`/`Unstable`な経路は明示的に除外する——物理的な修復は
/// できないが、少なくとも問題のある経路へ新規トラフィックを積極的に
/// 向けないという意味での「自動改善」。
pub fn recommend_healthiest<'a>(diagnoses: &'a [LinkDiagnosis]) -> Option<&'a LinkDiagnosis> {
    diagnoses
        .iter()
        .filter(|d| matches!(d.health, LinkHealth::Healthy))
        .min_by(|a, b| a.smoothed_rtt_ms.unwrap_or(f64::MAX).total_cmp(&b.smoothed_rtt_ms.unwrap_or(f64::MAX)))
        .or_else(|| {
            diagnoses
                .iter()
                .filter(|d| matches!(d.health, LinkHealth::Degraded))
                .min_by(|a, b| a.smoothed_rtt_ms.unwrap_or(f64::MAX).total_cmp(&b.smoothed_rtt_ms.unwrap_or(f64::MAX)))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn disconnected_interface_is_flagged_regardless_of_rtt_history() {
        let monitor = NetworkQualityMonitor::new();
        monitor.record_rtt(Duration::from_millis(5));
        let d = diagnose("eth0", false, &monitor);
        assert_eq!(d.health, LinkHealth::Disconnected);
    }

    #[test]
    fn high_jitter_is_flagged_as_unstable_even_with_low_mean_rtt() {
        let monitor = NetworkQualityMonitor::new();
        for ms in [5u64, 80, 3, 95, 10, 70, 2, 60] {
            monitor.record_rtt(Duration::from_millis(ms));
        }
        let d = diagnose("wifi0", true, &monitor);
        assert_eq!(d.health, LinkHealth::Unstable);
    }

    #[test]
    fn stable_low_rtt_is_healthy() {
        let monitor = NetworkQualityMonitor::new();
        for _ in 0..10 {
            monitor.record_rtt(Duration::from_millis(10));
        }
        let d = diagnose("eth1", true, &monitor);
        assert_eq!(d.health, LinkHealth::Healthy);
    }

    #[test]
    fn high_stable_rtt_is_degraded_not_unstable() {
        let monitor = NetworkQualityMonitor::new();
        for _ in 0..10 {
            monitor.record_rtt(Duration::from_millis(200));
        }
        let d = diagnose("wan0", true, &monitor);
        assert_eq!(d.health, LinkHealth::Degraded);
    }

    #[test]
    fn recommend_healthiest_prefers_healthy_over_degraded_and_skips_unstable() {
        let healthy_monitor = NetworkQualityMonitor::new();
        for _ in 0..10 {
            healthy_monitor.record_rtt(Duration::from_millis(10));
        }
        let degraded_monitor = NetworkQualityMonitor::new();
        for _ in 0..10 {
            degraded_monitor.record_rtt(Duration::from_millis(200));
        }
        let unstable_monitor = NetworkQualityMonitor::new();
        for ms in [5u64, 80, 3, 95, 10, 70] {
            unstable_monitor.record_rtt(Duration::from_millis(ms));
        }

        let diagnoses = vec![
            diagnose("wan-degraded", true, &degraded_monitor),
            diagnose("wifi-unstable", true, &unstable_monitor),
            diagnose("eth-healthy", true, &healthy_monitor),
        ];
        let best = recommend_healthiest(&diagnoses).unwrap();
        assert_eq!(best.name, "eth-healthy");
    }
}
