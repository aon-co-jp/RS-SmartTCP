//! 複数経路(有線LAN最大10本+WiFi)の中から最良経路を選ぶ+自動フェイル
//! オーバー(2026-08-11新設)。
//!
//! ユーザー指示「LANコネクターが仮にUSBであろうと、PCIE経由であろうと
//! マザーボード経由でもLANケーブルは最大10本＋Wifi同時接続で通信の
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
//! 1. **最良経路選択(高速化)**: 複数の経路(有線LAN最大10本+WiFi)
//!    それぞれのRTT/ジッター([`crate::NetworkQualityMonitor`])を
//!    個別に追跡し、新しい接続を張る際に最もRTTが低い経路を選ぶ。
//! 2. **自動フェイルオーバー(安定化)**: 選択中の経路が劣化・切断
//!    した場合、次に良い経路へ自動的に切り替える。

use std::collections::HashMap;
use std::sync::Mutex;

use crate::network_interfaces::{InterfaceKind, NetworkInterfaceReport};
use crate::NetworkQualityMonitor;

/// 同時に扱う経路数の目安(有線LAN最大10本、ユーザー指示の数値を
/// そのまま採用)。強制する上限ではなく、ドキュメント上の目安。
pub const MAX_WIRED_PATHS: usize = 10;

/// 同時に扱うWiFiチャンネル(回線)数の上限(2026-08-11追加、ユーザー
/// 指示「複数WiFiは最大10チャンネル(回線)…同時接続可能にして」)。
pub const MAX_WIFI_PATHS: usize = 10;

/// 同時に扱うBluetoothチャンネル(回線)数の上限(2026-08-11追加、
/// ユーザー指示「複数ブルーツースは最大10チャンネル(回線)…同時接続
/// 可能にして」)。
pub const MAX_BLUETOOTH_PATHS: usize = 10;

/// 経路の先につながる機器の種類(2026-08-11追加、ユーザー指示
/// 「ルーターと外付けHDDやNASなどに複数LANケーブル1本から最大10本＋
/// WiFiも追加可能にして対応して」+「PC、タブレット、スマホ、TV、
/// ゲームマシンなどとルーターと外付けHDDやNASなどに…対応して」への
/// 対応)。GUI/ログ上で意味のあるラベルを表示するための分類であり、
/// 実際の通信経路選択ロジック(`best_path`)自体はこの分類に関わらず
/// 全経路のRTTだけで判定する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Router,
    Nas,
    ExternalStorage,
    Pc,
    Tablet,
    Phone,
    Tv,
    GameConsole,
    Wifi,
    /// 2026-08-11追加(ユーザー指示「複数ブルーツゥース対応」)。
    Bluetooth,
    Other,
}

struct PathEntry {
    monitor: NetworkQualityMonitor,
    kind: DeviceKind,
    /// リンク速度(bps)の実測値(2026-08-11追加、ユーザー指示「経路
    /// コストの実測値化」への対応)。`network_interfaces`が検出できた
    /// 場合のみ`Some`——検出できない環境(非Windows等)では引き続き
    /// `None`のまま、呼び出し側が固定値へフォールバックする設計。
    link_speed_bps: Option<u64>,
    /// `path_optimizer`等の意思決定で「無効化」された経路かどうか
    /// (2026-08-11追加、ユーザー指示「最適化結果の実際のトラフィック
    /// 制御への反映」への対応)。既定`true`(有効)。`false`の経路は
    /// `best_path`の選択対象から除外される——物理的に接続を切るのでは
    /// なく、新規トラフィックをこの経路へ積極的に向けない、という
    /// ソフトウェア側の制御に留まる(正直な開示)。
    enabled: bool,
}

pub struct MultiPathManager {
    paths: Mutex<HashMap<String, PathEntry>>,
}

impl MultiPathManager {
    pub fn new() -> Self {
        Self { paths: Mutex::new(HashMap::new()) }
    }

    /// 検出済みのネットワークインターフェース([`crate::network_
    /// interfaces::detect`]の結果)から、接続中の有線LAN(最大
    /// [`MAX_WIRED_PATHS`]本)+WiFiを自動的に経路として登録した
    /// `MultiPathManager`を作る。ルーター・NAS・外付けHDD等、実際に
    /// 何につながっているかはこのクレートからは分からない
    /// (ローカルのネットワークインターフェースを見ているだけで、
    /// その先の機器の種類までは判別できない)ため、まずは`Other`種別で
    /// 登録し、呼び出し側が実際の接続先を把握していれば
    /// [`Self::register_device_path`]で後から種類を上書きできる。
    pub fn from_detected_interfaces(report: &NetworkInterfaceReport) -> Self {
        let mgr = Self::new();
        let mut wired_registered = 0usize;
        let mut wifi_registered = 0usize;
        let mut bluetooth_registered = 0usize;
        for iface in &report.interfaces {
            if !iface.connected {
                continue;
            }
            match iface.kind {
                InterfaceKind::Ethernet if wired_registered < MAX_WIRED_PATHS => {
                    mgr.register_device_path_with_speed(&iface.name, DeviceKind::Other, iface.link_speed_bps);
                    wired_registered += 1;
                }
                // WiFi・Bluetoothも、それぞれ独立した上限
                // (MAX_WIFI_PATHS/MAX_BLUETOOTH_PATHS=10)まで登録する
                // (2026-08-11、ユーザー指示「複数WiFiは最大10チャンネル
                // …複数ブルーツースは最大10チャンネル…同時接続可能に
                // して」)。
                InterfaceKind::Wifi if wifi_registered < MAX_WIFI_PATHS => {
                    mgr.register_device_path_with_speed(&iface.name, DeviceKind::Wifi, iface.link_speed_bps);
                    wifi_registered += 1;
                }
                InterfaceKind::Bluetooth if bluetooth_registered < MAX_BLUETOOTH_PATHS => {
                    mgr.register_device_path_with_speed(&iface.name, DeviceKind::Bluetooth, iface.link_speed_bps);
                    bluetooth_registered += 1;
                }
                _ => {}
            }
        }
        mgr
    }

    /// 経路(インターフェース名等の識別子)を登録する。既に存在する場合は
    /// 何もしない(冪等)。
    pub fn register_path(&self, name: &str) {
        self.register_device_path(name, DeviceKind::Other);
    }

    /// 経路を、先につながる機器の種類(ルーター・NAS・外付けHDD等)付きで
    /// 登録する。既に登録済みの経路名を指定した場合は種類だけを更新する
    /// (RTT測定値は保持したまま)。
    pub fn register_device_path(&self, name: &str, kind: DeviceKind) {
        self.register_device_path_with_speed(name, kind, None);
    }

    /// [`Self::register_device_path`]にリンク速度(bps)の実測値も添える
    /// バージョン(2026-08-11追加、ユーザー指示「経路コストの実測値化」
    /// への対応)。`link_speed_bps`が`Some`の場合、既存登録があっても
    /// 常に最新値へ更新する(接続後にリンク速度が変わることがあるため)。
    pub fn register_device_path_with_speed(&self, name: &str, kind: DeviceKind, link_speed_bps: Option<u64>) {
        let mut paths = self.paths.lock().unwrap();
        paths
            .entry(name.to_string())
            .and_modify(|e| {
                e.kind = kind;
                if link_speed_bps.is_some() {
                    e.link_speed_bps = link_speed_bps;
                }
            })
            .or_insert_with(|| PathEntry { monitor: NetworkQualityMonitor::new(), kind, link_speed_bps, enabled: true });
    }

    /// 指定した経路のRTTサンプルを記録する。未登録の経路名なら自動的に
    /// `Other`種別で登録する(呼び出し側が事前登録を忘れてもサービスを
    /// 壊さない)。
    pub fn record_rtt(&self, path_name: &str, rtt: std::time::Duration) {
        let mut paths = self.paths.lock().unwrap();
        paths
            .entry(path_name.to_string())
            .or_insert_with(|| PathEntry { monitor: NetworkQualityMonitor::new(), kind: DeviceKind::Other, link_speed_bps: None, enabled: true })
            .monitor
            .record_rtt(rtt);
    }

    /// 現時点で最もRTTが低い経路名を返す(サンプルが無い経路・
    /// [`Self::set_enabled`]で無効化された経路は除外)。登録済みの経路が
    /// 1つも無い、または対象になる経路が1つも無い場合は`None`
    /// (呼び出し側は既定の経路を使うこと)。
    pub fn best_path(&self) -> Option<String> {
        let paths = self.paths.lock().unwrap();
        paths
            .iter()
            .filter(|(_, entry)| entry.enabled)
            .filter_map(|(name, entry)| entry.monitor.smoothed_rtt_ms().map(|rtt| (name.clone(), rtt)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(name, _)| name)
    }

    /// 経路の有効/無効を切り替える(2026-08-11追加、ユーザー指示
    /// 「最適化結果の実際のトラフィック制御への反映」への対応)。
    /// `path_optimizer::optimize_path_selection`が「無効化すべき」と
    /// 判断した経路をここで実際に反映し、以後`best_path`の選択対象から
    /// 除外する。未登録の経路名を指定した場合は何もしない(`false`を
    /// 返す)。**正直な開示**: これは新規トラフィックの経路選択に
    /// 影響するのみで、物理的な接続を切断するものではない。
    pub fn set_enabled(&self, name: &str, enabled: bool) -> bool {
        let mut paths = self.paths.lock().unwrap();
        if let Some(entry) = paths.get_mut(name) {
            entry.enabled = enabled;
            true
        } else {
            false
        }
    }

    /// 指定した経路が現在有効かどうか(未登録の場合は`None`)。
    pub fn is_enabled(&self, name: &str) -> Option<bool> {
        self.paths.lock().unwrap().get(name).map(|e| e.enabled)
    }

    /// 現在登録されている経路数。
    pub fn path_count(&self) -> usize {
        self.paths.lock().unwrap().len()
    }

    /// 登録済みの全経路を(名前, 機器種別, 現在のSRTT〈msec、未計測なら
    /// None〉)のリストとして返す。GUIでの一覧表示用。
    pub fn registered_paths(&self) -> Vec<(String, DeviceKind, Option<f64>)> {
        let paths = self.paths.lock().unwrap();
        paths.iter().map(|(name, entry)| (name.clone(), entry.kind, entry.monitor.smoothed_rtt_ms())).collect()
    }

    /// [`Self::registered_paths`]に、実測リンク速度(bps、未検出なら
    /// `None`)も添えたバージョン(2026-08-11追加、経路コストの実測値化
    /// 用途——`path_optimizer`のコスト入力に実測値を渡せるようにする)。
    pub fn registered_paths_with_link_speed(&self) -> Vec<(String, DeviceKind, Option<f64>, Option<u64>)> {
        let paths = self.paths.lock().unwrap();
        paths.iter().map(|(name, entry)| (name.clone(), entry.kind, entry.monitor.smoothed_rtt_ms(), entry.link_speed_bps)).collect()
    }

    /// [`Self::registered_paths_with_link_speed`]に、現在の有効/無効
    /// 状態も添えたバージョン(GUI表示用)。
    pub fn registered_paths_with_status(&self) -> Vec<(String, DeviceKind, Option<f64>, Option<u64>, bool)> {
        let paths = self.paths.lock().unwrap();
        paths
            .iter()
            .map(|(name, entry)| (name.clone(), entry.kind, entry.monitor.smoothed_rtt_ms(), entry.link_speed_bps, entry.enabled))
            .collect()
    }

    /// [`crate::link_diagnostics`]向けの診断一覧を組み立てる。登録済み
    /// 経路それぞれについて、`report`側の接続状態と、内部の
    /// `NetworkQualityMonitor`が持つSRTT/RTTVARをスナップショットとして
    /// 渡す(`PathEntry`自体は非公開のため、値だけを取り出して橋渡しする
    /// 設計)。
    pub fn diagnose(&self, report: &crate::network_interfaces::NetworkInterfaceReport) -> Vec<crate::link_diagnostics::LinkDiagnosis> {
        let snapshots: Vec<(String, bool, Option<f64>, Option<f64>)> = {
            let paths = self.paths.lock().unwrap();
            paths
                .iter()
                .map(|(name, entry)| {
                    let connected = report.interfaces.iter().find(|i| &i.name == name).map(|i| i.connected).unwrap_or(true);
                    (name.clone(), connected, entry.monitor.smoothed_rtt_ms(), entry.monitor.rttvar_ms())
                })
                .collect()
        };
        crate::link_diagnostics::diagnose_all_from_snapshots(report, &snapshots)
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
    fn disabled_path_is_excluded_from_best_path_even_if_it_has_the_lowest_rtt() {
        let mgr = MultiPathManager::new();
        mgr.record_rtt("eth0", Duration::from_millis(50));
        mgr.record_rtt("eth1", Duration::from_millis(10));
        assert_eq!(mgr.best_path(), Some("eth1".to_string()), "eth1 must win before being disabled");

        assert!(mgr.set_enabled("eth1", false));
        assert_eq!(mgr.best_path(), Some("eth0".to_string()), "eth1 must be excluded once disabled, falling back to eth0");
        assert_eq!(mgr.is_enabled("eth1"), Some(false));
    }

    #[test]
    fn set_enabled_on_unregistered_path_returns_false_without_panicking() {
        let mgr = MultiPathManager::new();
        assert!(!mgr.set_enabled("nonexistent", false));
        assert_eq!(mgr.is_enabled("nonexistent"), None);
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

    #[test]
    fn register_device_path_labels_router_and_nas_without_affecting_selection() {
        let mgr = MultiPathManager::new();
        mgr.register_device_path("Router (LAN1)", DeviceKind::Router);
        mgr.register_device_path("NAS (LAN2)", DeviceKind::Nas);
        mgr.register_device_path("External HDD (LAN3)", DeviceKind::ExternalStorage);
        mgr.record_rtt("Router (LAN1)", Duration::from_millis(5));
        mgr.record_rtt("NAS (LAN2)", Duration::from_millis(20));
        mgr.record_rtt("External HDD (LAN3)", Duration::from_millis(15));

        let paths = mgr.registered_paths();
        assert_eq!(paths.len(), 3);
        assert!(paths.iter().any(|(name, kind, _)| name == "Router (LAN1)" && *kind == DeviceKind::Router));
        assert!(paths.iter().any(|(name, kind, _)| name == "NAS (LAN2)" && *kind == DeviceKind::Nas));
        assert_eq!(mgr.best_path(), Some("Router (LAN1)".to_string()));
    }

    #[test]
    fn from_detected_interfaces_registers_connected_wired_and_wifi_paths() {
        use crate::network_interfaces::{InterfaceKind, NetworkInterface, NetworkInterfaceReport};

        let report = NetworkInterfaceReport {
            interfaces: vec![
                NetworkInterface { name: "eth0".to_string(), kind: InterfaceKind::Ethernet, connected: true, link_speed_bps: None },
                NetworkInterface { name: "eth1".to_string(), kind: InterfaceKind::Ethernet, connected: false, link_speed_bps: None },
                NetworkInterface { name: "Wi-Fi".to_string(), kind: InterfaceKind::Wifi, connected: true, link_speed_bps: None },
                NetworkInterface { name: "Bluetooth".to_string(), kind: InterfaceKind::Other, connected: true, link_speed_bps: None },
            ],
        };
        let mgr = MultiPathManager::from_detected_interfaces(&report);
        // 接続済みのeth0とWi-Fiのみ登録される(未接続のeth1・Ethernet/Wifi
        // 以外のBluetoothは対象外)。
        assert_eq!(mgr.path_count(), 2);
        let paths = mgr.registered_paths();
        assert!(paths.iter().any(|(name, kind, _)| name == "eth0" && *kind == DeviceKind::Other));
        assert!(paths.iter().any(|(name, kind, _)| name == "Wi-Fi" && *kind == DeviceKind::Wifi));
    }

    #[test]
    fn from_detected_interfaces_registers_multiple_wifi_and_bluetooth() {
        use crate::network_interfaces::{InterfaceKind, NetworkInterface, NetworkInterfaceReport};

        let report = NetworkInterfaceReport {
            interfaces: vec![
                NetworkInterface { name: "Wi-Fi".to_string(), kind: InterfaceKind::Wifi, connected: true, link_speed_bps: None },
                NetworkInterface { name: "WiFi USB Dongle".to_string(), kind: InterfaceKind::Wifi, connected: true, link_speed_bps: None },
                NetworkInterface { name: "Bluetooth Network Connection".to_string(), kind: InterfaceKind::Bluetooth, connected: true, link_speed_bps: None },
                NetworkInterface { name: "Bluetooth PAN 2".to_string(), kind: InterfaceKind::Bluetooth, connected: true, link_speed_bps: None },
            ],
        };
        let mgr = MultiPathManager::from_detected_interfaces(&report);
        assert_eq!(mgr.path_count(), 4);
        let paths = mgr.registered_paths();
        assert_eq!(paths.iter().filter(|(_, k, _)| *k == DeviceKind::Wifi).count(), 2);
        assert_eq!(paths.iter().filter(|(_, k, _)| *k == DeviceKind::Bluetooth).count(), 2);
    }

    #[test]
    fn from_detected_interfaces_caps_wifi_and_bluetooth_at_ten_channels_each() {
        use crate::network_interfaces::{InterfaceKind, NetworkInterface, NetworkInterfaceReport};

        let mut interfaces: Vec<NetworkInterface> = (0..(MAX_WIFI_PATHS + 3))
            .map(|i| NetworkInterface { name: format!("wifi{i}"), kind: InterfaceKind::Wifi, connected: true, link_speed_bps: None })
            .collect();
        interfaces.extend((0..(MAX_BLUETOOTH_PATHS + 3)).map(|i| NetworkInterface {
            name: format!("bt{i}"),
            kind: InterfaceKind::Bluetooth,
            connected: true,
            link_speed_bps: None,
        }));
        let report = NetworkInterfaceReport { interfaces };
        let mgr = MultiPathManager::from_detected_interfaces(&report);
        let paths = mgr.registered_paths();
        assert_eq!(paths.iter().filter(|(_, k, _)| *k == DeviceKind::Wifi).count(), MAX_WIFI_PATHS, "must cap WiFi at 10 channels");
        assert_eq!(paths.iter().filter(|(_, k, _)| *k == DeviceKind::Bluetooth).count(), MAX_BLUETOOTH_PATHS, "must cap Bluetooth at 10 channels");
    }

    #[test]
    fn from_detected_interfaces_caps_wired_registrations_at_max_wired_paths() {
        use crate::network_interfaces::{InterfaceKind, NetworkInterface, NetworkInterfaceReport};

        let interfaces = (0..(MAX_WIRED_PATHS + 2))
            .map(|i| NetworkInterface { name: format!("eth{i}"), kind: InterfaceKind::Ethernet, connected: true, link_speed_bps: None })
            .collect();
        let report = NetworkInterfaceReport { interfaces };
        let mgr = MultiPathManager::from_detected_interfaces(&report);
        assert_eq!(mgr.path_count(), MAX_WIRED_PATHS, "must cap at the documented max of 10 wired paths");
    }
}
