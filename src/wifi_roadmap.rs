//! WiFi世代(WiFi4〜8)・周波数帯(2.4/5/6GHz)の組み合わせメタデータ
//! (2026-08-11新設、ユーザー指示「WiFiは2.4G/5G/6Gのいずれの組み合わせも、
//! WiFi4/5/6/7/8対応など将来のロードマップを考慮配慮して」への対応)。
//!
//! ## 正直な開示(最重要)
//!
//! 本モジュールが提供するのは**メタデータと上限管理のみ**——実際の
//! WiFi物理層(PHY)ネゴシエーション・変調方式・実効スループット測定は
//! OS/無線LANアダプタのドライバが行うものであり、本クレートが実装
//! できる対象ではない(このクレートの一貫した設計方針、
//! `network_interfaces.rs`が実際の接続状態を検出するのと同じ役割分担)。
//! ここで行っているのは、`multi_path`が管理する最大10チャンネルの
//! WiFi経路それぞれに「どの世代・どの周波数帯として扱うか」という
//! ラベル付け、および将来世代への拡張性を持たせた設計(下記ロードマップ
//! 節参照)。
//!
//! ## 世代×周波数帯の対応関係(2026-08-11、日英Web検索で裏取り)
//!
//! - **WiFi 4**(IEEE 802.11n、2009年): 2.4GHz/5GHz。
//! - **WiFi 5**(IEEE 802.11ac、2014年): 5GHzのみ。
//! - **WiFi 6**(IEEE 802.11ax、2019年): 2.4GHz/5GHz。
//! - **WiFi 6E**(同じ802.11ax、6GHz拡張、2020年代): 2.4GHz/5GHz/6GHz。
//! - **WiFi 7**(IEEE 802.11be、2024年正式化): 2.4GHz/5GHz/6GHz
//!   (Multi-Link Operationで複数帯域を同時使用可能)。
//! - **WiFi 8**(IEEE 802.11bn、**2026-08時点でドラフト段階**——
//!   Draft 1.0は2025年7月承認、最終標準化目標は2028年9月、消費者向け
//!   製品の広範な普及は2027〜2028年見込みという段階であり、まだ確定
//!   規格ではない): 2.4GHz/5GHz/6GHz、ピーク速度向上ではなく実効
//!   スループット・低レイテンシ・高信頼性を重視する設計思想
//!   ([Network World](https://www.networkworld.com/article/4112600/wi-fi-8-in-2026-next-gen-wireless-standard-prioritizes-reliability-over-speed-gains.html)、
//!   [Wikipedia: IEEE 802.11bn](https://en.wikipedia.org/wiki/IEEE_802.11bn))。
//!   **本クレートはWiFi 8を「将来のドラフト規格」として設定上受け入れ
//!   可能にしているが、実機がWiFi 8として動作することを保証するもの
//!   ではない**(規格自体が未確定のため)。
//!
//! ## フレッツ光クロス/IOWNとの関係(参考、本クレートの実装対象外)
//!
//! フレッツ光クロス(最大10Gbps光回線)向けレンタルルーターは
//! 2026年5月からWiFi 7対応機種の提供が始まっている
//! ([NTT東日本](https://www.ntt-east.co.jp/release/detail/20260330_02.html))。
//! IOWN/APN(NTTの光電融合ネットワーク基盤)は物理telecom基盤であり、
//! これも[`crate::NetworkQualityMonitor`]のモジュールdocコメントで
//! 既に明記した通り本クレートの実装対象ではない——ここでは「回線側が
//! 高速・低遅延になっても、WiFi側の世代・帯域の組み合わせを正しく
//! 扱えることが無駄にならない」という将来の前提を、設定上のメタデータ
//! として先取りしておく位置づけ。

use std::collections::HashMap;
use std::sync::Mutex;

/// WiFi世代。将来世代([`WifiGeneration::Wifi8`])は2026-08時点でドラフト
/// 段階の規格であることを、この列挙型のdocコメント・
/// [`WifiGeneration::is_finalized_standard`]の両方で常に明示する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WifiGeneration {
    /// IEEE 802.11n(2009年正式化)。
    Wifi4,
    /// IEEE 802.11ac(2014年正式化)。
    Wifi5,
    /// IEEE 802.11ax(2019年正式化)。
    Wifi6,
    /// IEEE 802.11ax の6GHz拡張(6E)。
    Wifi6E,
    /// IEEE 802.11be(2024年正式化)。
    Wifi7,
    /// IEEE 802.11bn。**2026-08時点でドラフト段階**(最終標準化目標
    /// 2028年9月)——正式規格ではない。
    Wifi8,
}

impl WifiGeneration {
    /// このセッション時点(2026-08-11)で最終標準化済みかどうか
    /// (誇張しない開示のため、ドラフト段階のWiFi 8のみ`false`)。
    pub fn is_finalized_standard(&self) -> bool {
        !matches!(self, WifiGeneration::Wifi8)
    }

    /// この世代が対応する周波数帯(IEEE仕様上の対応関係、実機の対応は
    /// アダプタ・ドライバ依存)。
    pub fn supported_bands(&self) -> &'static [WifiBand] {
        match self {
            WifiGeneration::Wifi4 => &[WifiBand::Band2_4Ghz, WifiBand::Band5Ghz],
            WifiGeneration::Wifi5 => &[WifiBand::Band5Ghz],
            WifiGeneration::Wifi6 => &[WifiBand::Band2_4Ghz, WifiBand::Band5Ghz],
            WifiGeneration::Wifi6E => &[WifiBand::Band2_4Ghz, WifiBand::Band5Ghz, WifiBand::Band6Ghz],
            WifiGeneration::Wifi7 => &[WifiBand::Band2_4Ghz, WifiBand::Band5Ghz, WifiBand::Band6Ghz],
            WifiGeneration::Wifi8 => &[WifiBand::Band2_4Ghz, WifiBand::Band5Ghz, WifiBand::Band6Ghz],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WifiBand {
    Band2_4Ghz,
    Band5Ghz,
    Band6Ghz,
}

/// 指定した世代・周波数帯の組み合わせが、IEEE仕様上サポートされて
/// いるか(実機の対応可否ではなく規格上の対応関係のチェック)。
pub fn is_valid_combination(generation: WifiGeneration, band: WifiBand) -> bool {
    generation.supported_bands().contains(&band)
}

/// `multi_path`が管理する最大10チャンネルのWiFi経路それぞれに、
/// 世代・周波数帯のラベルを付けるレジストリ。**これはIEEE仕様上有効な
/// 組み合わせかどうかの検証・記録のみを行い、実際にその世代/帯域で
/// 接続されていることを検出・保証するものではない**(冒頭の正直な
/// 開示参照)。
pub struct WifiChannelRegistry {
    channels: Mutex<HashMap<String, (WifiGeneration, WifiBand)>>,
}

impl WifiChannelRegistry {
    pub fn new() -> Self {
        Self { channels: Mutex::new(HashMap::new()) }
    }

    /// チャンネル(`multi_path`側のデバイス名と対応させる想定)へ
    /// 世代・周波数帯を設定する。IEEE仕様上無効な組み合わせ
    /// (例: WiFi 5+2.4GHz)は拒否する。
    pub fn set_channel(&self, name: &str, generation: WifiGeneration, band: WifiBand) -> Result<(), String> {
        if !is_valid_combination(generation, band) {
            return Err(format!(
                "{generation:?} does not support {band:?} per IEEE spec / {generation:?}は{band:?}に非対応(IEEE仕様上)"
            ));
        }
        self.channels.lock().unwrap().insert(name.to_string(), (generation, band));
        Ok(())
    }

    pub fn channel(&self, name: &str) -> Option<(WifiGeneration, WifiBand)> {
        self.channels.lock().unwrap().get(name).copied()
    }

    pub fn channel_count(&self) -> usize {
        self.channels.lock().unwrap().len()
    }
}

impl Default for WifiChannelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wifi8_is_flagged_as_not_yet_a_finalized_standard() {
        assert!(!WifiGeneration::Wifi8.is_finalized_standard());
        assert!(WifiGeneration::Wifi7.is_finalized_standard());
    }

    #[test]
    fn wifi5_only_supports_5ghz_not_2_4ghz() {
        assert!(!is_valid_combination(WifiGeneration::Wifi5, WifiBand::Band2_4Ghz));
        assert!(is_valid_combination(WifiGeneration::Wifi5, WifiBand::Band5Ghz));
    }

    #[test]
    fn wifi7_supports_all_three_bands() {
        for band in [WifiBand::Band2_4Ghz, WifiBand::Band5Ghz, WifiBand::Band6Ghz] {
            assert!(is_valid_combination(WifiGeneration::Wifi7, band));
        }
    }

    #[test]
    fn registry_rejects_invalid_combination_and_keeps_previous_state_unset() {
        let registry = WifiChannelRegistry::new();
        let result = registry.set_channel("wifi-1", WifiGeneration::Wifi5, WifiBand::Band2_4Ghz);
        assert!(result.is_err());
        assert!(registry.channel("wifi-1").is_none());
    }

    #[test]
    fn registry_accepts_valid_combination_and_can_be_read_back() {
        let registry = WifiChannelRegistry::new();
        registry.set_channel("wifi-1", WifiGeneration::Wifi6E, WifiBand::Band6Ghz).unwrap();
        assert_eq!(registry.channel("wifi-1"), Some((WifiGeneration::Wifi6E, WifiBand::Band6Ghz)));
        assert_eq!(registry.channel_count(), 1);
    }
}
