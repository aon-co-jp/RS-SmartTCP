//! 複数WAN回線対応(最大10本、2026-08-11新設)。
//!
//! ユーザー指示「複数WANも最大10本まで対応して」への対応。
//! [`crate::wan_config::WanConfig`](単一のWAN回線の設定: IPv4/IPv6/
//! v6プラス/自動設定)を複数本(名前付き、最大10本)管理する。
//!
//! ## 正直な開示
//!
//! 複数WAN回線を扱う場合の典型的な用途(負荷分散・冗長化)自体は
//! [`crate::multi_path::MultiPathManager`]の最良経路選択/自動
//! フェイルオーバーの仕組みをWAN回線名に対して使うことで実現できる
//! (LAN側の複数経路と同じ設計をそのまま流用できるため、本モジュール
//! では重複実装しない)。本モジュールが追加で提供するのは、WAN回線
//! ごとに**個別のIPv4/IPv6/v6プラス設定を持たせる**ための入れ物のみ。

use std::collections::HashMap;
use std::sync::Mutex;

use crate::wan_config::WanConfig;

/// 同時に扱えるWAN回線数の上限(ユーザー指示の数値をそのまま採用)。
pub const MAX_WAN_LINES: usize = 10;

pub struct MultiWanManager {
    lines: Mutex<HashMap<String, WanConfig>>,
}

impl MultiWanManager {
    pub fn new() -> Self {
        Self { lines: Mutex::new(HashMap::new()) }
    }

    /// WAN回線を登録する(既定設定はIPv4)。既に同名の回線があれば何も
    /// せず`Ok`を返す(冪等)。既に[`MAX_WAN_LINES`]本登録済みの場合は
    /// エラーを返す(黙って無視しない、正直な開示)。
    pub fn register_line(&self, name: &str) -> Result<(), String> {
        let mut lines = self.lines.lock().unwrap();
        if lines.contains_key(name) {
            return Ok(());
        }
        if lines.len() >= MAX_WAN_LINES {
            return Err(format!("cannot register more than {MAX_WAN_LINES} WAN lines / WAN回線は最大{MAX_WAN_LINES}本までです"));
        }
        lines.insert(name.to_string(), WanConfig::new());
        Ok(())
    }

    pub fn remove_line(&self, name: &str) {
        self.lines.lock().unwrap().remove(name);
    }

    pub fn line_count(&self) -> usize {
        self.lines.lock().unwrap().len()
    }

    pub fn line_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.lines.lock().unwrap().keys().cloned().collect();
        names.sort();
        names
    }

    /// 指定した回線に対して設定変更を行う。回線が未登録なら`false`を
    /// 返す(呼び出し側が誤ったWAN回線名を指定してもパニックしない)。
    pub fn set_auto_configure_enabled(&self, name: &str, enabled: bool) -> bool {
        self.with_line(name, |cfg| cfg.set_auto_configure_enabled(enabled))
    }

    pub fn set_ipv6_enabled(&self, name: &str, enabled: bool) -> bool {
        self.with_line(name, |cfg| cfg.set_ipv6_enabled(enabled))
    }

    pub fn set_v6_plus_enabled(&self, name: &str, enabled: bool) -> bool {
        self.with_line(name, |cfg| cfg.set_v6_plus_enabled(enabled))
    }

    pub fn connection_summary(&self, name: &str) -> Option<&'static str> {
        let lines = self.lines.lock().unwrap();
        lines.get(name).map(|cfg| cfg.connection_summary())
    }

    pub fn is_auto_configure_enabled(&self, name: &str) -> Option<bool> {
        let lines = self.lines.lock().unwrap();
        lines.get(name).map(|cfg| cfg.is_auto_configure_enabled())
    }

    pub fn is_ipv6_enabled(&self, name: &str) -> Option<bool> {
        let lines = self.lines.lock().unwrap();
        lines.get(name).map(|cfg| cfg.is_ipv6_enabled())
    }

    pub fn is_v6_plus_enabled(&self, name: &str) -> Option<bool> {
        let lines = self.lines.lock().unwrap();
        lines.get(name).map(|cfg| cfg.is_v6_plus_enabled())
    }

    fn with_line(&self, name: &str, f: impl FnOnce(&WanConfig)) -> bool {
        let lines = self.lines.lock().unwrap();
        match lines.get(name) {
            Some(cfg) => {
                f(cfg);
                true
            }
            None => false,
        }
    }
}

impl Default for MultiWanManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_and_configures_multiple_independent_wan_lines() {
        let mgr = MultiWanManager::new();
        mgr.register_line("WAN1 (Fiber A)").unwrap();
        mgr.register_line("WAN2 (Fiber B)").unwrap();

        mgr.set_v6_plus_enabled("WAN1 (Fiber A)", true);
        mgr.set_ipv6_enabled("WAN2 (Fiber B)", true);

        assert_eq!(mgr.connection_summary("WAN1 (Fiber A)"), Some("IPv6 (v6プラス / MAP-E)"));
        assert_eq!(mgr.connection_summary("WAN2 (Fiber B)"), Some("IPv6 (v6プラス以外 / non-MAP-E)"));
    }

    #[test]
    fn caps_registration_at_ten_lines() {
        let mgr = MultiWanManager::new();
        for i in 0..MAX_WAN_LINES {
            mgr.register_line(&format!("WAN{i}")).unwrap();
        }
        assert_eq!(mgr.line_count(), MAX_WAN_LINES);
        assert!(mgr.register_line("WAN10").is_err(), "must reject the 11th WAN line");
    }

    #[test]
    fn registering_the_same_name_twice_is_idempotent() {
        let mgr = MultiWanManager::new();
        mgr.register_line("WAN1").unwrap();
        mgr.register_line("WAN1").unwrap();
        assert_eq!(mgr.line_count(), 1);
    }

    #[test]
    fn operations_on_unregistered_line_return_none_or_false_without_panicking() {
        let mgr = MultiWanManager::new();
        assert!(!mgr.set_ipv6_enabled("does-not-exist", true));
        assert_eq!(mgr.connection_summary("does-not-exist"), None);
    }

    #[test]
    fn removing_a_line_frees_a_slot_for_a_new_one() {
        let mgr = MultiWanManager::new();
        for i in 0..MAX_WAN_LINES {
            mgr.register_line(&format!("WAN{i}")).unwrap();
        }
        mgr.remove_line("WAN0");
        assert!(mgr.register_line("WAN-new").is_ok());
    }
}
