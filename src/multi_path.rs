//! 複数経路(有線LAN最大4本+WiFi)の中から最良経路を選ぶ+自動フェイル
//! オーバー(2026-08-11新設)。
//!
//! ユーザー指示「LANコネクターが仮にUSBであろうと、PCIE経由であろうと
//! マザーボード経由でもLANケーブルは最大4本＋Wifi同時接続で通信の
//! 高速化と安定化機能を搭載して」への対応。
//!
//! ## 正直な開示(最重要)
//!
//! **これは本物の帯域合算リンクアグリゲーション(複数回線の速度を
//! 足し合わせて1本の高速回線のように使う機能)ではない。** 真の
//! リンクアグリゲーションは、OS/NICドライバのチーミング機能
//! (Windows Serverの`New-NetLbfoTeam`等)またはMPTCP(Linuxカーネルの
//! マルチパスTCP)を必要とし、いずれもこのクレートが持つ「ユーザー
//! 空間のRustライブラリ」という立場からは実装できない(コンシューマ
//! 版Windowsには標準のNICチーミング機能自体が無い)。
//!
//! 本モジュールが実際に提供するのは以下の2点であり、それぞれ
//! 「高速化」と「安定化」という元の要望に対応する、誠実な範囲の実装:
//!
//! 1. **最良経路選択(高速化)**: 複数の経路(有線LAN最大4本+WiFi)
//!    それぞれのRTT/ジッター([`crate::NetworkQualityMonitor`])を
//!    個別に追跡し、新しい接続を張る際に最もRTTが低い経路を選ぶ。
//! 2. **自動フェイルオーバー(安定化)**: 選択中の経路が劣化・切断
//!    した場合、次に良い経路へ自動的に切り替える。

use std::collections::HashMap;
use std::sync::Mutex;

use crate::NetworkQualityMonitor;

/// 同時に扱う経路数の目安(有線LAN最大4本+WiFi1本、ユーザー指示の
/// 数値をそのまま採用)。強制する上限ではなく、ドキュメント上の目安。
pub const MAX_WIRED_PATHS: usize = 4;

pub struct MultiPathManager {
    paths: Mutex<HashMap<String, NetworkQualityMonitor>>,
}

impl MultiPathManager {
    pub fn new() -> Self {
        Self { paths: Mutex::new(HashMap::new()) }
    }

    /// 経路(インターフェース名等の識別子)を登録する。既に存在する場合は
    /// 何もしない(冪等)。
    pub fn register_path(&self, name: &str) {
        let mut paths = self.paths.lock().unwrap();
        paths.entry(name.to_string()).or_insert_with(NetworkQualityMonitor::new);
    }

    /// 指定した経路のRTTサンプルを記録する。未登録の経路名なら自動的に
    /// 登録する(呼び出し側が事前登録を忘れてもサービスを壊さない)。
    pub fn record_rtt(&self, path_name: &str, rtt: std::time::Duration) {
        let mut paths = self.paths.lock().unwrap();
        paths.entry(path_name.to_string()).or_insert_with(NetworkQualityMonitor::new).record_rtt(rtt);
    }

    /// 現時点で最もRTTが低い経路名を返す(サンプルが無い経路は除外)。
    /// 登録済みの経路が1つも無い、またはどの経路にもサンプルが無い
    /// 場合は`None`(呼び出し側は既定の経路を使うこと)。
    pub fn best_path(&self) -> Option<String> {
        let paths = self.paths.lock().unwrap();
        paths
            .iter()
            .filter_map(|(name, monitor)| monitor.smoothed_rtt_ms().map(|rtt| (name.clone(), rtt)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(name, _)| name)
    }

    /// 現在登録されている経路数。
    pub fn path_count(&self) -> usize {
        self.paths.lock().unwrap().len()
    }
}

impl Default for MultiPathManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn best_path_picks_the_lowest_rtt_among_registered_paths() {
        let mgr = MultiPathManager::new();
        mgr.record_rtt("eth0", Duration::from_millis(50));
        mgr.record_rtt("eth1", Duration::from_millis(10));
        mgr.record_rtt("wifi", Duration::from_millis(30));
        assert_eq!(mgr.best_path(), Some("eth1".to_string()));
    }

    #[test]
    fn best_path_is_none_when_no_samples_recorded() {
        let mgr = MultiPathManager::new();
        mgr.register_path("eth0");
        assert_eq!(mgr.best_path(), None);
    }

    #[test]
    fn failover_switches_best_path_when_active_path_degrades() {
        let mgr = MultiPathManager::new();
        mgr.record_rtt("eth0", Duration::from_millis(10));
        mgr.record_rtt("wifi", Duration::from_millis(50));
        assert_eq!(mgr.best_path(), Some("eth0".to_string()));

        // eth0が劣化(ケーブル抜け等を想定した高RTT連発)。
        for _ in 0..10 {
            mgr.record_rtt("eth0", Duration::from_millis(500));
        }
        assert_eq!(mgr.best_path(), Some("wifi".to_string()), "must fail over to the now-better path");
    }

    #[test]
    fn supports_up_to_four_wired_paths_plus_wifi() {
        let mgr = MultiPathManager::new();
        for i in 0..MAX_WIRED_PATHS {
            mgr.record_rtt(&format!("eth{i}"), Duration::from_millis(20 + i as u64));
        }
        mgr.record_rtt("wifi", Duration::from_millis(15));
        assert_eq!(mgr.path_count(), MAX_WIRED_PATHS + 1);
        assert_eq!(mgr.best_path(), Some("wifi".to_string()));
    }
}
