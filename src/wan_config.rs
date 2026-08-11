//! WAN接続設定(IPv4/IPv6/IPv6 v6プラス〈MAP-E〉+自動設定機能、
//! 2026-08-11新設)。
//!
//! ユーザー指示「IPV4からIPV6からV6プラス対応にして、WANからの接続を
//! 自動設定機能+V6プラス機能にチェックボックスを付けたり外したりが
//! 可能にして。V6でもV6プラス以外の接続も可能にして」への対応。
//!
//! ## 正直な開示(最重要)
//!
//! **これは実際にDHCPv6-PD交渉・MAP-E(v6プラス)のパラメータ取得・
//! トンネル確立を行う本物のWAN接続実装ではない。** v6プラスのような
//! ISP固有の移行方式は、実際にはルーター機器のファームウェアまたは
//! OSのネットワークスタック(Windowsの場合はNCSI/ネットワークアダプタ
//! ドライバ層)が行う処理であり、ユーザー空間のRustライブラリからは
//! 実装できない。本モジュールが提供するのは、**ユーザーが選んだ接続
//! 方式の意図を表す設定フラグ**のみであり、実際のWAN接続確立は別途
//! OS/ルーター機器側の設定が必要である。

use std::sync::atomic::{AtomicBool, Ordering};

pub struct WanConfig {
    auto_configure_enabled: AtomicBool,
    ipv6_enabled: AtomicBool,
    v6_plus_enabled: AtomicBool,
}

impl WanConfig {
    pub fn new() -> Self {
        Self {
            auto_configure_enabled: AtomicBool::new(false),
            ipv6_enabled: AtomicBool::new(false),
            v6_plus_enabled: AtomicBool::new(false),
        }
    }

    pub fn set_auto_configure_enabled(&self, enabled: bool) {
        self.auto_configure_enabled.store(enabled, Ordering::SeqCst);
    }

    pub fn is_auto_configure_enabled(&self) -> bool {
        self.auto_configure_enabled.load(Ordering::SeqCst)
    }

    pub fn set_ipv6_enabled(&self, enabled: bool) {
        self.ipv6_enabled.store(enabled, Ordering::SeqCst);
        if !enabled {
            // IPv6自体を無効化したら、その上に乗るv6プラスも意味を失う
            // ため一緒に無効化する(矛盾した状態を保持しない)。
            self.v6_plus_enabled.store(false, Ordering::SeqCst);
        }
    }

    pub fn is_ipv6_enabled(&self) -> bool {
        self.ipv6_enabled.load(Ordering::SeqCst)
    }

    /// v6プラス(MAP-E)を有効化する。ユーザー指示「V6でもV6プラス以外の
    /// 接続も可能にして」に対応するため、v6プラスは独立したON/OFFの
    /// トグルとして扱う——OFFにしても`ipv6_enabled`はそのまま(＝
    /// 「IPv6は使うがv6プラスではない」状態になる)。ONにする場合は
    /// IPv6自体も自動的に有効化する(v6プラスはIPv6前提の方式のため)。
    pub fn set_v6_plus_enabled(&self, enabled: bool) {
        self.v6_plus_enabled.store(enabled, Ordering::SeqCst);
        if enabled {
            self.ipv6_enabled.store(true, Ordering::SeqCst);
        }
    }

    pub fn is_v6_plus_enabled(&self) -> bool {
        self.v6_plus_enabled.load(Ordering::SeqCst)
    }

    /// 現在の接続方式を人間が読める形で返す(GUI表示用)。
    pub fn connection_summary(&self) -> &'static str {
        if !self.is_ipv6_enabled() {
            "IPv4"
        } else if self.is_v6_plus_enabled() {
            "IPv6 (v6プラス / MAP-E)"
        } else {
            "IPv6 (v6プラス以外 / non-MAP-E)"
        }
    }
}

impl Default for WanConfig {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_ipv4() {
        let wan = WanConfig::new();
        assert_eq!(wan.connection_summary(), "IPv4");
    }

    #[test]
    fn enabling_v6_plus_also_enables_ipv6() {
        let wan = WanConfig::new();
        wan.set_v6_plus_enabled(true);
        assert!(wan.is_ipv6_enabled());
        assert_eq!(wan.connection_summary(), "IPv6 (v6プラス / MAP-E)");
    }

    #[test]
    fn ipv6_without_v6_plus_is_a_valid_independent_state() {
        let wan = WanConfig::new();
        wan.set_ipv6_enabled(true);
        assert!(!wan.is_v6_plus_enabled(), "IPv6 must be usable without v6 plus");
        assert_eq!(wan.connection_summary(), "IPv6 (v6プラス以外 / non-MAP-E)");
    }

    #[test]
    fn disabling_ipv6_also_disables_v6_plus() {
        let wan = WanConfig::new();
        wan.set_v6_plus_enabled(true);
        wan.set_ipv6_enabled(false);
        assert!(!wan.is_v6_plus_enabled(), "v6 plus cannot remain on when IPv6 itself is off");
        assert_eq!(wan.connection_summary(), "IPv4");
    }

    #[test]
    fn auto_configure_toggle_is_independent_of_ip_version_choice() {
        let wan = WanConfig::new();
        wan.set_auto_configure_enabled(true);
        wan.set_v6_plus_enabled(true);
        assert!(wan.is_auto_configure_enabled());
        wan.set_v6_plus_enabled(false);
        assert!(wan.is_auto_configure_enabled(), "toggling v6 plus must not affect the auto-configure flag");
    }
}
