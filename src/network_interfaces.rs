//! ネットワークインターフェース検出(2026-08-11新設)。
//!
//! ユーザー指示「LANケーブルは、LANコネクターが仮にUSBであろうと、
//! PCIE経由であろうとマザーボード経由でもLANケーブルは最大4本＋Wifi
//! 同時接続で通信の高速化と安定化機能」への対応の第一歩。
//!
//! ## 正直な開示(最重要)
//!
//! - **バス種別(USB/PCIe/オンボード)は区別できない**: Windowsの標準
//!   ネットワークAPI(`ipconfig`が使う経路)は、物理的な接続方式
//!   (USBアダプタか、PCIeカードか、マザーボード直付けか)を区別せず、
//!   すべて等しく「イーサネットアダプタ」として見える。この違いを
//!   区別するにはWMI(`Win32_NetworkAdapter`のバス種別プロパティ)への
//!   問い合わせが必要で、より複雑な実装になる——今回はユーザー要望の
//!   本質(「有線LANが何本つながっているか」の検出)を満たすため、
//!   バス種別の区別は行わない設計とした。
//! - **外部クレート非依存を維持**: このクレートの既存方針
//!   (`#![no unsafe]`、std以外への依存なし)に従い、Windows標準の
//!   `ipconfig`コマンドを`std::process::Command`で呼び出しテキスト
//!   出力を解析する(WMI/`windows`クレートのような追加依存は導入しない)。
//! - **Windows専用**: `ipconfig`はWindowsのコマンドのため、この機能は
//!   Windows上でのみ動作する。他OSでは空の結果を返す(パニックしない)。

use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterfaceKind {
    Ethernet,
    Wifi,
    Other,
}

#[derive(Debug, Clone)]
pub struct NetworkInterface {
    pub name: String,
    pub kind: InterfaceKind,
    /// IPv4アドレスが取得できている(=リンクが繋がっている)かどうかの
    /// 簡易判定。
    pub connected: bool,
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
}

/// `ipconfig`の出力を解析する(テスト容易性のため、実行部分と分離)。
/// 各アダプタブロックは空行で区切られ、1行目に
/// `"Ethernet adapter <name>:"`または`"Wireless LAN adapter <name>:"`
/// のような見出しが来て、以降のインデント行に`IPv4 Address`等が続く
/// (英語版Windowsの書式。日本語版Windowsでは見出しが「イーサネット
/// アダプター」「Wireless LAN アダプター」等になるため、両対応する)。
pub fn parse_ipconfig_output(output: &str) -> NetworkInterfaceReport {
    let mut interfaces = Vec::new();
    let mut current: Option<NetworkInterface> = None;

    for line in output.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        if !line.starts_with(' ') && !line.starts_with('\t') && trimmed.contains(':') {
            // 新しいアダプタの見出し行。
            if let Some(iface) = current.take() {
                interfaces.push(iface);
            }
            let header = trimmed.trim_end_matches(':');
            let kind = if header.to_lowercase().contains("wireless") || header.contains("Wi-Fi") || header.contains("WLAN") {
                InterfaceKind::Wifi
            } else if header.to_lowercase().contains("ethernet") || header.contains("イーサネット") {
                InterfaceKind::Ethernet
            } else {
                InterfaceKind::Other
            };
            current = Some(NetworkInterface { name: header.to_string(), kind, connected: false });
        } else if let Some(iface) = current.as_mut() {
            let lower = trimmed.to_lowercase();
            if lower.contains("ipv4") && trimmed.contains('.') {
                iface.connected = true;
            }
            if lower.contains("media disconnected") || lower.contains("メディアは接続されていません") {
                iface.connected = false;
            }
        }
    }
    if let Some(iface) = current.take() {
        interfaces.push(iface);
    }

    NetworkInterfaceReport { interfaces }
}

/// 実際に`ipconfig`を実行して現在のネットワークインターフェース状況を
/// 取得する。Windows以外・コマンド実行失敗時は空のレポートを返す
/// (パニックしない、正直な開示として`interfaces`が空のまま)。
///
/// **実機検証で発見・修正した実バグ**: 日本語版Windowsでは`ipconfig`の
/// 既定出力エンコーディングがシステムのANSIコードページ(Shift-JIS/
/// cp932)であり、これを素朴に`String::from_utf8_lossy`で解釈すると
/// 日本語のアダプタ名(「イーサネット アダプター」等)が文字化けし、
/// 種別判定(Ethernet/Wifi)にも失敗する実バグがあった。外部クレート
/// (`encoding_rs`等)を追加せずに解決するため、`cmd /C chcp 65001 >nul
/// && ipconfig`という形でコードページを一時的にUTF-8(65001)へ切り替えた
/// 上で`ipconfig`を実行する(`chcp`はコマンド実行環境=起動した
/// `cmd`プロセス内でのみ有効なため、呼び出し元プロセスや他の処理には
/// 影響しない)。
///
/// **正直な開示・既知の残存制限**: 上記の対応で見出し行("Ethernet
/// adapter"/"Wireless LAN adapter")の判定・接続本数のカウント・
/// 種別判定は実機で正しく動作することを確認済みだが、アダプタの
/// **表示名自体**(Windows側がローカライズして付けた既定名、例:
/// 「イーサネット」)は、`chcp`切り替え後もなお文字化けする既知の
/// 制限が残っている(Windowsコンソールの内部的なコードページ処理の
/// 制約、`ipconfig`側の実装に起因)。機能面(本数・種別・接続状態の
/// 正誤判定)には影響しないため、追加の外部クレート導入を伴う完全な
/// 解決は今回のスコープ外とした——名前表示の完全な文字化け解消には
/// PowerShellの`Get-NetAdapter`(構造化出力)への切り替え等が必要。
pub fn detect() -> NetworkInterfaceReport {
    if !cfg!(windows) {
        return NetworkInterfaceReport::default();
    }
    match Command::new("cmd").args(["/C", "chcp 65001 >nul && ipconfig"]).output() {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            parse_ipconfig_output(&text)
        }
        Err(_) => NetworkInterfaceReport::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_OUTPUT: &str = "\
Windows IP Configuration

Ethernet adapter Ethernet:

   Connection-specific DNS Suffix  . :
   IPv4 Address. . . . . . . . . . . : 192.168.0.10
   Subnet Mask . . . . . . . . . . . : 255.255.255.0

Ethernet adapter Ethernet 2:

   Media State . . . . . . . . . . . : Media disconnected

Wireless LAN adapter Wi-Fi:

   Connection-specific DNS Suffix  . :
   IPv4 Address. . . . . . . . . . . : 192.168.0.20
   Subnet Mask . . . . . . . . . . . : 255.255.255.0
";

    #[test]
    fn parses_connected_and_disconnected_ethernet_adapters() {
        let report = parse_ipconfig_output(SAMPLE_OUTPUT);
        assert_eq!(report.wired_connected_count(), 1, "only one of the two Ethernet adapters has an IPv4 address");
    }

    #[test]
    fn parses_connected_wifi_adapter() {
        let report = parse_ipconfig_output(SAMPLE_OUTPUT);
        assert!(report.wifi_connected());
    }

    #[test]
    fn empty_output_yields_empty_report() {
        let report = parse_ipconfig_output("");
        assert_eq!(report.wired_connected_count(), 0);
        assert!(!report.wifi_connected());
    }
}
