//! ルーターアプリ機能・セキュリティルーター機能のチェックボックス+
//! 追加プラグイン(2026-08-11新設)。
//!
//! ユーザー指示「ルーターアプリ機能＋セキュリティルーター機能のそれ
//! ぞれにチェックを付けられる様にして、チェックを付けると追加インス
//! トールのプラグインを追加インストール可能にして」への対応。
//!
//! ## 正直な開示(最重要・セキュリティ配慮)
//!
//! **これは任意の外部コード(バイナリ・スクリプト)をダウンロード・
//! 実行する本物のプラグイン機構ではない。** 未知のコードを実行する
//! 仕組みは、このクレートが使われる文脈(ホームルーター/セキュリティ
//! ゲートウェイ)においては特に重大なセキュリティリスク(サプライ
//! チェーン攻撃・任意コード実行)となるため、意図的に実装しない。
//!
//! 代わりに提供するのは、**あらかじめこのクレートに組み込まれた
//! 既知の機能モジュール一覧から選んで有効/無効を切り替える**、
//! 「プラグイン風」の機能フラグ管理である。新しい機能を追加するには
//! このクレート自体のソースコードを更新する必要があり、実行時に
//! 外部から任意のコードを注入することはできない。

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginInfo {
    pub id: &'static str,
    pub label_en: &'static str,
    pub label_ja: &'static str,
}

/// 「ルーターアプリ機能」チェックを入れた場合に選べる、既知の機能
/// モジュール一覧(誇張しない範囲の代表例)。
pub const ROUTER_APP_PLUGINS: &[PluginInfo] = &[
    PluginInfo { id: "port_forwarding", label_en: "Port forwarding", label_ja: "ポート転送" },
    PluginInfo { id: "qos", label_en: "QoS / bandwidth prioritization", label_ja: "QoS(帯域の優先制御)" },
    PluginInfo { id: "dhcp_server", label_en: "DHCP server", label_ja: "DHCPサーバー" },
];

/// 「セキュリティルーター機能」チェックを入れた場合に選べる、既知の
/// 機能モジュール一覧(誇張しない範囲の代表例、いずれも既存の
/// シグネチャ/ルールベース方式を想定——高度なAI検知は別途実装が必要)。
pub const SECURITY_ROUTER_PLUGINS: &[PluginInfo] = &[
    PluginInfo { id: "ad_tracker_blocking", label_en: "Ad & tracker blocking", label_ja: "広告・トラッカーブロック" },
    PluginInfo { id: "dns_filtering", label_en: "DNS filtering", label_ja: "DNSフィルタリング" },
    PluginInfo { id: "parental_controls", label_en: "Parental controls", label_ja: "ペアレンタルコントロール" },
];

pub struct RouterFeatures {
    router_app_enabled: AtomicBool,
    security_router_enabled: AtomicBool,
    installed_plugins: Mutex<HashSet<String>>,
}

impl RouterFeatures {
    pub fn new() -> Self {
        Self {
            router_app_enabled: AtomicBool::new(false),
            security_router_enabled: AtomicBool::new(false),
            installed_plugins: Mutex::new(HashSet::new()),
        }
    }

    pub fn set_router_app_enabled(&self, enabled: bool) {
        self.router_app_enabled.store(enabled, Ordering::SeqCst);
        if !enabled {
            self.uninstall_all(ROUTER_APP_PLUGINS);
        }
    }

    pub fn is_router_app_enabled(&self) -> bool {
        self.router_app_enabled.load(Ordering::SeqCst)
    }

    pub fn set_security_router_enabled(&self, enabled: bool) {
        self.security_router_enabled.store(enabled, Ordering::SeqCst);
        if !enabled {
            self.uninstall_all(SECURITY_ROUTER_PLUGINS);
        }
    }

    pub fn is_security_router_enabled(&self) -> bool {
        self.security_router_enabled.load(Ordering::SeqCst)
    }

    fn uninstall_all(&self, plugins: &[PluginInfo]) {
        let mut installed = self.installed_plugins.lock().unwrap();
        for p in plugins {
            installed.remove(p.id);
        }
    }

    /// 既知のプラグインIDを有効化する。対応する親機能(ルーターアプリ/
    /// セキュリティルーター)がまだ有効化されていない場合、または
    /// 未知のIDの場合はエラーを返す(黙って無視しない、正直な開示)。
    pub fn install_plugin(&self, id: &str) -> Result<(), String> {
        if ROUTER_APP_PLUGINS.iter().any(|p| p.id == id) {
            if !self.is_router_app_enabled() {
                return Err(format!("'{id}' requires the router app function to be enabled first / '{id}'にはルーターアプリ機能を先に有効化してください"));
            }
        } else if SECURITY_ROUTER_PLUGINS.iter().any(|p| p.id == id) {
            if !self.is_security_router_enabled() {
                return Err(format!("'{id}' requires the security router function to be enabled first / '{id}'にはセキュリティルーター機能を先に有効化してください"));
            }
        } else {
            return Err(format!("unknown plugin id: {id} / 不明なプラグインID: {id}"));
        }
        self.installed_plugins.lock().unwrap().insert(id.to_string());
        Ok(())
    }

    pub fn uninstall_plugin(&self, id: &str) {
        self.installed_plugins.lock().unwrap().remove(id);
    }

    pub fn is_plugin_installed(&self, id: &str) -> bool {
        self.installed_plugins.lock().unwrap().contains(id)
    }

    pub fn installed_plugins(&self) -> Vec<String> {
        let mut v: Vec<String> = self.installed_plugins.lock().unwrap().iter().cloned().collect();
        v.sort();
        v
    }
}

impl Default for RouterFeatures {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_install_requires_parent_feature_enabled_first() {
        let features = RouterFeatures::new();
        let err = features.install_plugin("qos").unwrap_err();
        assert!(err.contains("router app function"));

        features.set_router_app_enabled(true);
        assert!(features.install_plugin("qos").is_ok());
        assert!(features.is_plugin_installed("qos"));
    }

    #[test]
    fn security_router_plugin_requires_security_router_enabled() {
        let features = RouterFeatures::new();
        assert!(features.install_plugin("dns_filtering").is_err());
        features.set_security_router_enabled(true);
        assert!(features.install_plugin("dns_filtering").is_ok());
    }

    #[test]
    fn unknown_plugin_id_is_rejected() {
        let features = RouterFeatures::new();
        features.set_router_app_enabled(true);
        features.set_security_router_enabled(true);
        assert!(features.install_plugin("totally_made_up_plugin").is_err());
    }

    #[test]
    fn disabling_parent_feature_uninstalls_its_plugins() {
        let features = RouterFeatures::new();
        features.set_router_app_enabled(true);
        features.install_plugin("port_forwarding").unwrap();
        assert!(features.is_plugin_installed("port_forwarding"));

        features.set_router_app_enabled(false);
        assert!(!features.is_plugin_installed("port_forwarding"), "disabling the parent feature must uninstall its plugins");
    }

    #[test]
    fn independent_toggles_do_not_affect_each_other() {
        let features = RouterFeatures::new();
        features.set_router_app_enabled(true);
        features.install_plugin("qos").unwrap();
        features.set_security_router_enabled(true);
        features.install_plugin("dns_filtering").unwrap();

        features.set_security_router_enabled(false);
        assert!(features.is_plugin_installed("qos"), "router app plugins must survive toggling the unrelated security router feature");
        assert!(!features.is_plugin_installed("dns_filtering"));
    }
}
