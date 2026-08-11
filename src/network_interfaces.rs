//! ネットワークインターフェース検出(2026-08-11新設)。
//!
//! ユーザー指示「LANケーブルは、LANコネクターが仮にUSBであろうと、
//! PCIE経由であろうとマザーボード経由でもLANケーブルは最大4本＋Wifi
//! 同時接続で通信の高速化と安定化機能」への対応の第一歩。
//!
//! ## 正直な開示(最重要)
//!
//! - **バス種別(USB/PCIe/オンボード)は区別できない**: Windowsの標準
//!   ネットワークAPIは、物理的な接続方式(USBアダプタか、PCIeカードか、
//!   マザーボード直付けか)を区別せず、すべて等しく「イーサネット
//!   アダプタ」として見える。この違いを区別するにはWMIのバス種別
//!   プロパティへの問い合わせが必要で、より複雑な実装になる——今回は
//!   ユーザー要望の本質(「有線LANが何本つながっているか」の検出)を
//!   満たすため、バス種別の区別は行わない設計とした。
//! - **外部クレート非依存を維持**: このクレートの既存方針
//!   (`#![no unsafe]`、std以外への依存なし)に従い、Windows標準の
//!   PowerShell(`Get-NetAdapter`)を`std::process::Command`で呼び出し
//!   テキスト出力を解析する(`windows`クレートのような追加依存は
//!   導入しない)。
//! - **Windows専用**: この機能はWindows上でのみ動作する。他OSでは
//!   空の結果を返す(パニックしない)。
//!
//! ## 実機検証で発見・修正した実バグ(`ipconfig`解析 → `Get-NetAdapter`へ変更)
//!
//! 当初`ipconfig`のテキスト出力を解析していたが、実機検証(日本語版
//! Windows)で「イーサネットアダプターの文字の下の行などが全ての行で
//! 文字化けして読めません」という報告があった。原因は`ipconfig`の
//! 既定出力がシステムのANSIコードページ(Shift-JIS/cp932)であり、
//! `chcp 65001`でコンソールのコードページを切り替えても、`ipconfig`
//! 自身がローカライズ済みのアダプタ表示名(「イーサネット」等)を
//! 内部的に別経路でレンダリングするためか、文字化けが解消しなかった
//! こと。**根本対応として、`ipconfig`のテキスト解析自体をやめ、
//! PowerShellの`Get-NetAdapter`が返す構造化データ(`Status`・
//! `PhysicalMediaType`)を使う方式へ変更した**——`PhysicalMediaType`は
//! `"802.3"`(有線イーサネット)・`"Native 802.11"`(WiFi)のような
//! **英数字の技術定数**であり、Windows表示言語に関わらず値が変わらない
//! ため、ロケール依存の文字化け問題自体が原理的に起こらない。アダプタ
//! 名(`Name`)も`[Console]::OutputEncoding`をUTF-8へ明示的に設定した
//! 上でPowerShellから取得するため、日本語名でも正しく読める。

use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterfaceKind {
    Ethernet,
    Wifi,
    /// 2026-08-11追加(ユーザー指示「複数LAN＋複数WiFi＋複数ブルー
    /// ツゥース対応」への対応)。Bluetoothネットワーク接続アダプタ
    /// (`PhysicalMediaType`が`"Bluetooth"`)を区別する。
    Bluetooth,
    Other,
}

#[derive(Debug, Clone)]
pub struct NetworkInterface {
    pub name: String,
    pub kind: InterfaceKind,
    pub connected: bool,
    /// リンク速度(bps、`Get-NetAdapter`の`LinkSpeed`プロパティの実測値、
    /// 未接続・取得不可の場合は`None`)。2026-08-11追加、ユーザー指示
    /// 「経路コストの実測値化」への対応——契約帯域や実測スループットの
    /// 直接計測ではないが、OSが実際に報告するリンク速度という実測値。
    pub link_speed_bps: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct NetworkInterfaceReport {
    pub interfaces: Vec<NetworkInterface>,
}

impl NetworkInterfaceReport {
    pub fn wired_connected_count(&self) -> usize {
        self.interfaces.iter().filter(|i| i.kind == InterfaceKind::Ethernet && i.connected).count()
    }

    pub fn wifi_connected(&self) -> bool {
        self.interfaces.iter().any(|i| i.kind == InterfaceKind::Wifi && i.connected)
    }

    /// 接続中のWiFiアダプタの本数(2026-08-11追加、ユーザー指示「複数
    /// WiFi対応」——内蔵WiFi+USB WiFiドングル等、複数枚刺さっている
    /// 環境向け)。
    pub fn wifi_connected_count(&self) -> usize {
        self.interfaces.iter().filter(|i| i.kind == InterfaceKind::Wifi && i.connected).count()
    }

    /// 接続中のBluetoothネットワークアダプタの本数(2026-08-11追加、
    /// ユーザー指示「複数ブルーツゥース対応」)。
    pub fn bluetooth_connected_count(&self) -> usize {
        self.interfaces.iter().filter(|i| i.kind == InterfaceKind::Bluetooth && i.connected).count()
    }
}

/// `Get-NetAdapter`の1行区切りテキスト出力(`Name||Status||
/// PhysicalMediaType`形式、`detect()`が生成するコマンドの出力形式)を
/// 解析する(テスト容易性のため実行部分と分離)。
pub fn parse_netadapter_output(output: &str) -> NetworkInterfaceReport {
    let mut interfaces = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parts: Vec<&str> = trimmed.split("||").collect();
        if parts.len() < 3 {
            continue;
        }
        let (name, status, media_type) = (parts[0].trim(), parts[1].trim(), parts[2].trim());
        let link_speed_bps = parts.get(3).and_then(|s| s.trim().parse::<u64>().ok()).filter(|&v| v > 0);
        let media_lower = media_type.to_lowercase();
        let kind = if media_lower.contains("802.11") || media_lower.contains("native 802.11") {
            InterfaceKind::Wifi
        } else if media_lower.contains("802.3") {
            InterfaceKind::Ethernet
        } else if media_lower.contains("bluetooth") {
            InterfaceKind::Bluetooth
        } else {
            InterfaceKind::Other
        };
        let connected = status.eq_ignore_ascii_case("up");
        interfaces.push(NetworkInterface { name: name.to_string(), kind, connected, link_speed_bps });
    }
    NetworkInterfaceReport { interfaces }
}

/// 実際に`Get-NetAdapter`を実行して現在のネットワークインターフェース
/// 状況を取得する。Windows以外・コマンド実行失敗時は空のレポートを
/// 返す(パニックしない、正直な開示として`interfaces`が空のまま)。
pub fn detect() -> NetworkInterfaceReport {
    if !cfg!(windows) {
        return NetworkInterfaceReport::default();
    }
    let script = "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; \
Get-NetAdapter | ForEach-Object { $_.Name + '||' + $_.Status + '||' + $_.PhysicalMediaType + '||' + $_.Speed }";
    match Command::new("powershell").args(["-NoProfile", "-Command", script]).output() {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            parse_netadapter_output(&text)
        }
        Err(_) => NetworkInterfaceReport::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_OUTPUT: &str = "\
Ethernet||Up||802.3
Ethernet 2||Disconnected||802.3
Ethernet 3||Up||802.3
Wi-Fi||Disconnected||Native 802.11
";

    #[test]
    fn parses_connected_and_disconnected_ethernet_adapters() {
        let report = parse_netadapter_output(SAMPLE_OUTPUT);
        assert_eq!(report.wired_connected_count(), 2, "two of the three Ethernet adapters are Up");
    }

    #[test]
    fn parses_wifi_adapter_by_physical_media_type_not_connected() {
        let report = parse_netadapter_output(SAMPLE_OUTPUT);
        assert!(!report.wifi_connected());
    }

    #[test]
    fn parses_connected_wifi_adapter() {
        let report = parse_netadapter_output("Wi-Fi||Up||Native 802.11\n");
        assert!(report.wifi_connected());
    }

    #[test]
    fn empty_output_yields_empty_report() {
        let report = parse_netadapter_output("");
        assert_eq!(report.wired_connected_count(), 0);
        assert!(!report.wifi_connected());
    }

    #[test]
    fn classifies_bluetooth_network_adapters_and_counts_multiple_wifi() {
        let report = parse_netadapter_output(
            "Wi-Fi||Up||Native 802.11\nWiFi USB Dongle||Up||Native 802.11\nBluetooth Network Connection||Up||Bluetooth\n",
        );
        assert_eq!(report.wifi_connected_count(), 2, "must count both WiFi adapters");
        assert_eq!(report.bluetooth_connected_count(), 1);
    }

    #[test]
    fn preserves_japanese_adapter_names_without_mojibake() {
        let report = parse_netadapter_output("イーサネット||Up||802.3\n");
        assert_eq!(report.interfaces[0].name, "イーサネット");
    }
}
