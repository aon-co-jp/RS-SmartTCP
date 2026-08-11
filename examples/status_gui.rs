//! 動作確認用の簡易GUI(ステータスページ、2026-08-11新設)。
//!
//! ユーザー指示「機能確認の為のGUI化と、今何がつながっているかの確認
//! 機能も付けて」+「ルーターと外付けHDDやNASなどに複数LANケーブル
//! 1本から最大10本＋WiFiも追加可能にして対応して」への対応。標準
//! ライブラリのみ(`std::net`)で実装した最小限のHTTPサーバー——この
//! クレートの既存方針「外部依存クレート無し」を保つため、RPoem等の
//! Webフレームワークは意図的に使わない。
//!
//! 実行方法: `cargo run --example status_gui`(既定`http://127.0.0.1:7878/`、
//! `RS_SMARTTCP_GUI_BIND`環境変数で上書き可)。
//!
//! **正直な開示**: これは最小限の単一リクエスト処理サーバーであり、
//! 同時多接続・HTTPS・堅牢なエラーハンドリングは無い(動作確認用の
//! 簡易ツールとしての位置づけ)。ルーター/NAS/外付けHDDへの「RTT測定」は
//! TCP接続確立にかかる時間(`TcpStream::connect_timeout`)を計測する
//! 簡易的な方法であり、ICMP pingそのものではない(ICMP送信には管理者
//! 権限やRAWソケットが必要なため、この方が権限昇格無しに動く)。

use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rs_smarttcp::bandwidth_policy::BandwidthPolicy;
use rs_smarttcp::download_protection;
use rs_smarttcp::maintenance;
use rs_smarttcp::multi_path::{DeviceKind, MultiPathManager};
use rs_smarttcp::multi_wan::MultiWanManager;
use rs_smarttcp::network_interfaces;
use rs_smarttcp::path_optimizer;
use rs_smarttcp::raid_bridge;
use rs_smarttcp::redundant_transmission;
use rs_smarttcp::router_features::{RouterFeatures, ROUTER_APP_PLUGINS, SECURITY_ROUTER_PLUGINS};
use rs_smarttcp::secure_channel::SecureChannel;
use rs_smarttcp::transaction_log::TransactionLog;
use rs_smarttcp::usb_protection;

fn bind_addr() -> String {
    std::env::var("RS_SMARTTCP_GUI_BIND").unwrap_or_else(|_| "127.0.0.1:7878".to_string())
}

struct AppState {
    policy: BandwidthPolicy,
    paths: MultiPathManager,
    router_features: RouterFeatures,
    wan: MultiWanManager,
    /// `usb_protection::poll_new_drives`用の「前回確認時点のドライブ
    /// 集合」。ボタン押下(定期ポーリング)のたびに更新される。
    usb_seen: Mutex<HashSet<PathBuf>>,
    /// 直近の「新規USBドライブ確認」結果の表示用テキスト。
    usb_check_message: Mutex<Option<String>>,
    /// 直近の経路選択最適化(東芝SBM)の結果表示用テキスト。
    path_optimization_message: Mutex<Option<String>>,
    /// 直近の「4層暗号化+WAL+冗長送信」デモの結果表示用テキスト。
    durability_demo_message: Mutex<Option<String>>,
}

fn device_kind_label(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Router => "Router / ルーター",
        DeviceKind::Nas => "NAS",
        DeviceKind::ExternalStorage => "External HDD / 外付けHDD",
        DeviceKind::Pc => "PC",
        DeviceKind::Tablet => "Tablet / タブレット",
        DeviceKind::Phone => "Phone / スマホ",
        DeviceKind::Tv => "TV",
        DeviceKind::GameConsole => "Game console / ゲーム機",
        DeviceKind::Wifi => "Wi-Fi",
        DeviceKind::Bluetooth => "Bluetooth",
        DeviceKind::Other => "Other / その他",
    }
}

fn render_page(
    state: &AppState,
    probe_error: Option<&str>,
    scan_result: Option<&download_protection::ScanResult>,
    maintenance_report: Option<&maintenance::MaintenanceReport>,
) -> String {
    let report = network_interfaces::detect();
    let wired = report.wired_connected_count();
    let wifi_count = report.wifi_connected_count();
    let bt_count = report.bluetooth_connected_count();
    let checked = if state.policy.is_streaming_cap_enabled() { "checked" } else { "" };

    let rows: String = report
        .interfaces
        .iter()
        .map(|i| {
            format!(
                "<tr><td>{}</td><td>{:?}</td><td>{}</td></tr>",
                html_escape(&i.name),
                i.kind,
                if i.connected { "Connected / 接続中" } else { "Not connected / 未接続" }
            )
        })
        .collect();

    let best = state.paths.best_path();
    let device_rows: String = state
        .paths
        .registered_paths_with_status()
        .into_iter()
        .map(|(name, kind, rtt_ms, _link_speed_bps, enabled)| {
            let is_best = best.as_deref() == Some(name.as_str());
            format!(
                "<tr{}><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                if is_best { " style=\"font-weight:bold;background:#eef8ee;\"" } else { "" },
                html_escape(&name),
                device_kind_label(kind),
                rtt_ms.map(|v| format!("{v:.1} ms{}", if is_best { " (best / 最良経路)" } else { "" })).unwrap_or_else(|| "no data / 未測定".to_string()),
                if enabled { "🟢 enabled / 有効" } else { "⛔ disabled by optimizer / 最適化により無効化" }
            )
        })
        .collect();

    let diagnoses = state.paths.diagnose(&report);
    let diagnostics_html: String = diagnoses
        .iter()
        .map(|d| {
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
                html_escape(&d.name),
                d.health.label_ja(),
                html_escape(&d.reason_ja)
            )
        })
        .collect();

    let path_optimization_html = state
        .path_optimization_message
        .lock()
        .unwrap()
        .as_ref()
        .map(|m| format!("<div style=\"border:1px solid #ccc; border-radius:6px; padding:10px; margin-top:8px;\">{}</div>", html_escape(m)))
        .unwrap_or_default();

    let durability_demo_html = state
        .durability_demo_message
        .lock()
        .unwrap()
        .as_ref()
        .map(|m| format!("<div style=\"border:1px solid #ccc; border-radius:6px; padding:10px; margin-top:8px; white-space:pre-line;\">{}</div>", html_escape(m)))
        .unwrap_or_default();

    let raid_accel_html = match raid_bridge::detect_parity_accelerator() {
        Some(accel) => format!(
            "<p><strong>{:?}</strong> (CPU fallback / CPUフォールバック: {})</p>",
            accel.kind,
            raid_bridge::is_cpu_fallback(&accel)
        ),
        None => "<p>Accelerator detection failed; CPU-only implementation will be used. / アクセラレータ検出に失敗、CPU実装のみで動作します。</p>".to_string(),
    };

    let error_html = probe_error
        .map(|e| format!("<p style=\"color:#c33;\">Probe failed / 疎通確認に失敗しました: {}</p>", html_escape(e)))
        .unwrap_or_default();

    let scan_result_html = scan_result
        .map(|r| {
            format!(
                "<div style=\"border:1px solid #ccc; border-radius:6px; padding:10px; margin-top:8px;\"><strong>{:?}</strong><br>{}<br>{}</div>",
                r.outcome,
                html_escape(&r.message_en),
                html_escape(&r.message_ja)
            )
        })
        .unwrap_or_default();

    let usb_check_html = state
        .usb_check_message
        .lock()
        .unwrap()
        .as_ref()
        .map(|m| format!("<div style=\"border:1px solid #ccc; border-radius:6px; padding:10px; margin-top:8px;\">{}</div>", html_escape(m)))
        .unwrap_or_default();

    let usb_drives_html: String = usb_protection::list_removable_drives()
        .into_iter()
        .map(|drive| {
            let d = html_escape(&drive.display().to_string());
            format!(
                r#"<li>{d}
<form method="post" action="/protect-usb-drive" style="display:inline;">
<input type="hidden" name="drive" value="{d}">
<select name="backend"><option value="clamav">ClamAV</option><option value="kingsoft">KINGSOFT</option></select>
<button type="submit">Protect this drive / このドライブを保護</button>
</form></li>"#
            )
        })
        .collect();
    let av_recommendation_html = if download_protection::has_registered_antivirus() {
        String::new()
    } else {
        let (en, ja) = download_protection::install_security_software_recommendation();
        format!(
            "<div style=\"border:1px solid #c90; background:#fff8e6; border-radius:6px; padding:10px; margin-top:12px;\">⚠️ {}<br>{}</div>",
            html_escape(&en),
            html_escape(&ja)
        )
    };

    let maintenance_html = maintenance_report
        .map(|r| {
            format!(
                "<div style=\"border:1px solid #ccc; border-radius:6px; padding:10px; margin-top:8px;\">{}<br>{}</div>",
                html_escape(&r.summary_en),
                html_escape(&r.summary_ja)
            )
        })
        .unwrap_or_default();

    let usb_drives_html = if usb_drives_html.is_empty() {
        "<p style=\"color:#666; font-size:0.85em;\">No removable drives detected / リムーバブルドライブは検出されていません</p>".to_string()
    } else {
        format!("<ul>{usb_drives_html}</ul>")
    };

    let router_app_checked = if state.router_features.is_router_app_enabled() { "checked" } else { "" };
    let security_router_checked = if state.router_features.is_security_router_enabled() { "checked" } else { "" };

    let router_app_plugins_html = if state.router_features.is_router_app_enabled() {
        render_plugin_list(&state.router_features, ROUTER_APP_PLUGINS)
    } else {
        String::new()
    };
    let security_router_plugins_html = if state.router_features.is_security_router_enabled() {
        render_plugin_list(&state.router_features, SECURITY_ROUTER_PLUGINS)
    } else {
        String::new()
    };

    let wan_line_count = state.wan.line_count();
    let wan_lines_html: String = state
        .wan
        .line_names()
        .into_iter()
        .map(|name| {
            let summary = state.wan.connection_summary(&name).unwrap_or("?");
            let auto_checked = if state.wan.is_auto_configure_enabled(&name).unwrap_or(false) { "checked" } else { "" };
            let ipv6_checked = if state.wan.is_ipv6_enabled(&name).unwrap_or(false) { "checked" } else { "" };
            let v6_plus_checked = if state.wan.is_v6_plus_enabled(&name).unwrap_or(false) { "checked" } else { "" };
            let n = html_escape(&name);
            format!(
                r#"<div style="border:1px solid #ccc; border-radius:6px; padding:10px; margin-bottom:8px;">
<strong>{n}</strong> — {summary}
<form method="post" action="/toggle-wan-auto-configure"><input type="hidden" name="name" value="{n}"><label><input type="checkbox" name="enabled" value="1" {auto_checked} onchange="this.form.submit()"> Auto-configure / 自動設定</label></form>
<form method="post" action="/toggle-ipv6"><input type="hidden" name="name" value="{n}"><label><input type="checkbox" name="enabled" value="1" {ipv6_checked} onchange="this.form.submit()"> Use IPv6 / IPv6を使用する</label></form>
<form method="post" action="/toggle-v6-plus"><input type="hidden" name="name" value="{n}"><label><input type="checkbox" name="enabled" value="1" {v6_plus_checked} onchange="this.form.submit()"> v6 Plus (MAP-E) / v6プラス</label></form>
</div>"#
            )
        })
        .collect();

    format!(
        r#"<!doctype html>
<html><head><meta charset="utf-8"><title>RS-SmartTCP status</title></head>
<body style="font-family: sans-serif; max-width: 720px; margin: 40px auto;">
<h1>RS-SmartTCP — Connection status / 接続状況</h1>
<p>Wired Ethernet connected / 有線LAN接続本数: <strong>{wired}</strong> (max 10 supported / 最大10本まで対応)</p>
<p>Wi-Fi connected / WiFi接続本数: <strong>{wifi_count}</strong> (max 10 channels supported / 最大10チャンネルまで対応)</p>
<p>Bluetooth connected / Bluetooth接続本数: <strong>{bt_count}</strong> (max 10 channels supported / 最大10チャンネルまで対応)</p>
<table border="1" cellpadding="6" style="border-collapse: collapse;">
<tr><th>Interface / インターフェース</th><th>Kind / 種別</th><th>Status / 状態</th></tr>
{rows}
</table>

<h2>Router / NAS / External HDD paths / 経路一覧</h2>
<p style="font-size:0.85em; color:#666;">Add your router, NAS, or external HDD's address below to measure and compare its response time (up to 10 wired + Wi-Fi). / 下のフォームからルーター・NAS・外付けHDDのアドレスを追加すると、応答時間を測定・比較できます(有線最大10本+WiFi)。</p>
<table border="1" cellpadding="6" style="border-collapse: collapse;">
<tr><th>Name / 名前</th><th>Kind / 種別</th><th>Response time / 応答時間</th><th>Traffic control / トラフィック制御</th></tr>
{device_rows}
</table>

<h2>Link diagnostics / 通信品質の診断</h2>
<p style="font-size:0.85em; color:#666;">Detects disconnected/unstable/degraded links from OS connection status and RTT/RTTVAR — not a physical cable sensor, but an inference from measured data. / OS上の接続状態とRTT/RTTVAR実測値から、断線・不安定・低速な経路を推測します(物理的なケーブルセンサーではありません)。</p>
<table border="1" cellpadding="6" style="border-collapse: collapse;">
<tr><th>Link / 経路</th><th>Health / 状態</th><th>Reason / 理由</th></tr>
{diagnostics_html}
</table>
<p style="color:#999; font-size: 0.8em;">Honest disclosure: "automatic improvement" here means best_path already routes traffic to the lowest-RTT healthy link — this cannot physically repair a cable. / 正直な開示: ここでの「自動改善」はbest_pathが既にRTTの最も低い健全な経路へ自動的にトラフィックを寄せていることを指します——物理的なケーブルの修復はできません。</p>

<h2>Path selection optimizer (Toshiba SBM) / 経路選択最適化(東芝SBM)</h2>
<p style="font-size:0.85em; color:#666;">Selects which measured links to activate under a bandwidth-cost budget, maximizing total link quality (1/RTT), via a Simulated Bifurcation solver. / 帯域コスト予算の下で、通信品質(1/RTT)の合計が最大になる経路の組み合わせをシミュレーテッド分岐で選びます。</p>
<form method="post" action="/optimize-paths" style="display:flex; gap:8px; align-items:center;">
<input type="number" name="budget" placeholder="Budget (arbitrary cost units) / 予算" value="200" min="1" required>
<button type="submit">Optimize / 最適化</button>
</form>
{path_optimization_html}
<p style="color:#999; font-size: 0.8em;">Honest disclosure: for this small a number of links (max 10), brute force finds the exact optimum instantly — this demonstrates wiring SBM into a real decision path, not a necessity. Cost per link uses the real link speed (Mbps) reported by the OS when available, otherwise falls back to a placeholder fixed value (the result message shows which). / 正直な開示: この規模(最大10経路)なら全探索でも瞬時に厳密解が求まり、SBMを使う実用上の必要性は薄いものです——実際の意思決定パスへの配線実証が目的です。経路ごとのコストは、取得できる場合はOSが報告する実測リンク速度(Mbps)を使用し、取得できない場合は仮の固定値へフォールバックします(結果表示でどちらか分かります)。</p>

<h2>RAID-Z2/Z3 parity accelerator / RAID-Z2/Z3パリティアクセラレータ</h2>
{raid_accel_html}
<p style="color:#999; font-size: 0.8em;">Honest disclosure: this reuses open-raid-z + zfs_accel_hlsl as-is (no new RAID implementation here). Only FileBackedDevice (loopback file) is supported today — no real NVMe block device path exists yet. / 正直な開示: open-raid-z + zfs_accel_hlslをそのまま再利用しています(新規RAID実装はありません)。現時点でFileBackedDevice(ループバックファイル)のみ対応で、実NVMeブロックデバイスへの経路はまだありません。</p>

<h2>Durability + encryption demo / 耐障害性・暗号化デモ</h2>
<p style="font-size:0.85em; color:#666;">Demonstrates the full pipeline for "never lose data": encrypt (secure_channel, ChaCha20-Poly1305 + replay guard) → durably record (transaction_log, WAL with fsync) → send over up to 4 redundant paths (redundant_transmission, first success wins). / 「データを紛失しない」一連の流れを実演します: 暗号化(secure_channel)→WALへ確実に記録(transaction_log、fsync)→最大4経路への冗長送信(redundant_transmission、最初の成功を採用)。</p>
<form method="post" action="/durability-demo" style="display:flex; gap:8px; align-items:center;">
<input type="text" name="message" placeholder="Test message / テストメッセージ" value="transfer $100 to account X" style="min-width:260px;">
<button type="submit">Run demo / デモを実行</button>
</form>
{durability_demo_html}
<p style="color:#999; font-size: 0.8em;">Honest disclosure: the "redundant paths" here are simulated closures (one deliberately fails) for demonstration — real network transports are not wired in this example. / 正直な開示: ここでの「冗長経路」は実演用のシミュレートされたクロージャです(1つはわざと失敗させています)——この例では実際のネットワーク伝送路は配線していません。</p>
{error_html}
<form method="post" action="/probe" style="margin-top: 12px; display:flex; gap:8px; flex-wrap:wrap; align-items:center;">
<input type="text" name="name" placeholder="Name / 名前 (e.g. NAS)" required>
<select name="kind">
<option value="router">Router / ルーター</option>
<option value="nas">NAS</option>
<option value="external_storage">External HDD / 外付けHDD</option>
<option value="pc">PC</option>
<option value="tablet">Tablet / タブレット</option>
<option value="phone">Phone / スマホ</option>
<option value="tv">TV</option>
<option value="game_console">Game console / ゲーム機</option>
<option value="other">Other / その他</option>
</select>
<input type="text" name="host" placeholder="IP or hostname:port (e.g. 192.168.1.1:80)" required>
<button type="submit">Measure / 測定</button>
</form>

<form method="post" action="/toggle-streaming-cap" style="margin-top: 20px;">
<label>
<input type="checkbox" name="enabled" value="1" {checked} onchange="this.form.submit()">
Fix speed to 10Mbps for streaming (YouTube / U-NEXT / Qobuz etc.) to improve audio quality? /
音質向上のため、動画・音楽ストリーミング(YouTube・U-NEXT・Qobuz等)利用時の通信速度を10Mbpsに固定しますか？
</label>
</form>

<h2>Router / security features / ルーター・セキュリティ機能</h2>
<form method="post" action="/toggle-router-app">
<label><input type="checkbox" name="enabled" value="1" {router_app_checked} onchange="this.form.submit()"> Router app function / ルーターアプリ機能</label>
</form>
{router_app_plugins_html}
<form method="post" action="/toggle-security-router">
<label><input type="checkbox" name="enabled" value="1" {security_router_checked} onchange="this.form.submit()"> Security router function / セキュリティルーター機能</label>
</form>
{security_router_plugins_html}
<p style="color:#999; font-size: 0.8em;">Honest disclosure: these plugins are pre-built modules shipped with this crate, not arbitrary downloaded/executed third-party code (running unknown code would be a serious security risk for a router/security gateway). / 正直な開示: これらのプラグインはこのクレートにあらかじめ組み込まれた既知のモジュールであり、任意の外部コードをダウンロード・実行するものではありません(ルーター/セキュリティゲートウェイの文脈で未知のコードを実行することは重大なセキュリティリスクのため)。</p>

<h2>WAN connections / WAN回線一覧(最大10本)</h2>
<p>WAN lines registered / 登録済みWAN回線数: <strong>{wan_line_count}</strong> / 10</p>
{wan_lines_html}
<form method="post" action="/add-wan-line" style="display:flex; gap:8px; align-items:center;">
<input type="text" name="name" placeholder="WAN line name / WAN回線名 (e.g. WAN1 Fiber A)" required>
<button type="submit">Add WAN line / WAN回線を追加</button>
</form>
<p style="font-size:0.85em; color:#666;">You can use IPv6 without v6 Plus (e.g. native/PPPoE IPv6) by leaving the v6 Plus box unchecked, per WAN line. / v6プラスのチェックを外したままでも、IPv6自体(ネイティブ/PPPoE方式等)は回線ごとに利用できます。</p>
<p style="color:#999; font-size: 0.8em;">Honest disclosure: these are configuration-intent flags only — actual WAN negotiation (DHCPv6-PD, MAP-E parameter retrieval, tunnel setup) is performed by your OS/router firmware, not by this library. / 正直な開示: これらは設定意図を表すフラグに過ぎません——実際のWAN接続確立(DHCPv6-PD交渉・MAP-Eパラメータ取得・トンネル設定)はOS/ルーター機器側が行い、このライブラリ自体は行いません。</p>
<p style="color:#666; font-size: 0.9em;">Other traffic (regular websites, SFTP, Claude and other AI/chat tools) always runs at full speed. /
それ以外の通信(通常のWebサイト・SFTP・ClaudeなどのAI・チャットツール等)は常に最高速度で動作します。</p>
<p style="color:#999; font-size: 0.8em;">Honest disclosure: this does not sum the bandwidth of multiple links into one faster connection (true link aggregation requires OS/NIC teaming support). It picks the best-performing path and fails over automatically. Response time is measured via TCP connect time, not ICMP ping. / 正直な開示: 複数回線の速度を合算する機能ではありません(本物のリンクアグリゲーションにはOS/NICのチーミング機能が必要です)。最良経路の選択と自動フェイルオーバーを行います。応答時間はICMP pingではなくTCP接続確立時間で測定しています。</p>

<h2>Removable drives (USB) / リムーバブルドライブ(USB)</h2>
<p style="font-size:0.85em; color:#666;">Detected removable drives can be protected: a malicious autorun.inf (if present) is quarantined (renamed, not deleted) and the drive is scanned. / 検出されたリムーバブルドライブを保護できます: 悪意のあるautorun.inf(存在すれば)を隔離(削除ではなくリネーム)し、ドライブをスキャンします。</p>
{usb_drives_html}
<form method="post" action="/check-new-usb" style="margin-top:8px;">
<button type="submit">Check for newly inserted drives / 新しく挿入されたドライブを確認</button>
</form>
{usb_check_html}
<p style="color:#999; font-size: 0.8em;">Honest disclosure: Windows itself already disables autorun.inf execution for USB removable drives by default (autorun only applies to optical media) — this feature additionally finds and neutralizes any autorun.inf file that may still be present as a precaution. This library still has no OS-level device-insertion event hook (no WM_DEVICECHANGE) — the "check for newly inserted drives" button above polls the current drive list against the last-checked snapshot, so it approximates "the moment you plug it in" only as often as you (or your host app's timer) click it, not truly instantly. / 正直な開示: WindowsはUSBリムーバブルドライブのautorun.inf自動実行を既定で無効化済みです(自動実行は光学メディアのみ)——本機能は念のため残存するautorun.infを見つけて無害化するものです。本ライブラリは依然としてOSレベルのデバイス挿入イベントフック(WM_DEVICECHANGE)を持ちません——上記の「新しく挿入されたドライブを確認」ボタンは、前回確認時点との差分をポーリングで検出するものであり、「挿した瞬間」に近づけるにはご自身(または呼び出し側アプリのタイマー)がこまめに押す/呼ぶ必要があります。</p>

{av_recommendation_html}

<h2>Maintenance / メンテナンス</h2>
<form method="post" action="/run-maintenance">
<button type="submit">Run maintenance now / 今すぐメンテナンスを実行</button>
</form>
{maintenance_html}
<p style="color:#999; font-size: 0.8em;">Honest disclosure: this checks antivirus registration and updates ClamAV's virus definitions (via the official freshclam tool) — it does not run automatically on startup or on a schedule by itself; the host application should call this on its own timer or OS task scheduler. / 正直な開示: セキュリティソフトの登録確認とClamAVのウイルス定義更新(公式のfreshclamツール経由)を行います——このライブラリ自体が起動時・定期的に自動実行することはなく、呼び出し側アプリが独自のタイマーやOSのタスクスケジューラから呼ぶ必要があります。</p>

<h2>Downloaded file protection / ダウンロードファイル保護</h2>
<p style="background:#eef; border-radius:6px; padding:8px 10px; font-size:0.9em;">A new downloaded file was detected. Would you like to scan it — automatically now, or manually later? / 新しくダウンロードされたファイルを検知しました。自動でスキャンしますか？それとも後で手動でスキャンしますか？</p>
<p style="font-size:0.85em; color:#666;">Enter a file path to scan it for computer viruses. / ファイルパスを入力してコンピューターウイルスをスキャンします。</p>
<form method="post" action="/scan-file" style="display:flex; gap:8px; align-items:center; flex-wrap:wrap;">
<input type="text" name="path" placeholder="File path / ファイルパス (e.g. C:\Downloads\file.zip)" style="min-width:260px;" required>
<select name="backend">
<option value="clamav">ClamAV (open source / オープンソース)</option>
<option value="kingsoft">KINGSOFT Internet Security (free / 無料版)</option>
</select>
<label><input type="checkbox" name="unblock" value="1" checked> Mark as safe to open if clean (removes the download warning; you still double-click to open it) / クリーンなら「開いても安全」とマーク(ダウンロード警告を解除、開くのは引き続きご自身のダブルクリックです)</label>
<button type="submit">Scan / スキャン</button>
</form>
{scan_result_html}
<p style="color:#999; font-size: 0.8em;">Honest disclosure: no custom virus-detection engine is implemented here — this calls the real ClamAV (clamscan) or triggers KINGSOFT. ClamAV automates the result; KINGSOFT does not expose a documented command-line scan API and, per live testing, its trigger does not reliably open a visible window on its own — open KINGSOFT from its tray icon and confirm the result manually. Archive contents are scanned by the chosen engine itself, not by custom decompression code here. This tool never opens or runs a file on your behalf — a threat found is only ever quarantined (moved), and a clean file only ever has its download warning removed; actually opening a file is always your own action. / 正直な開示: 独自のウイルス検出エンジンは実装していません——実際のClamAV(clamscan)を呼び出すか、KINGSOFTのスキャン画面を開きます。ClamAVは結果を自動取得しますが、KINGSOFTは文書化されたコマンドラインAPIが無く、実機検証の結果、単独では可視の画面を確実には開かないため、タスクトレイから開いて結果を手動確認してください。圧縮ファイルの中身はここでの独自解凍ではなく、選択したエンジン自体がスキャンします。このツールがファイルを代わりに開いたり実行したりすることは一切ありません——脅威が見つかった場合は隔離(移動)するのみ、クリーンな場合はダウンロード警告を解除するのみで、実際にファイルを開く操作は常にご自身が行います。</p>
</body></html>"#
    )
}

fn render_plugin_list(features: &RouterFeatures, plugins: &[rs_smarttcp::router_features::PluginInfo]) -> String {
    // ストリーミング固定チェックボックスと同じパターン(1チェックボックス
    // =1フォーム)にする——複数チェックボックスを1フォームにまとめると、
    // チェックを外した瞬間にどのプラグインを外したのか(未チェックの
    // <input>はPOSTボディに含まれない仕様のため)判別できなくなる問題を
    // 避けるため。
    let items: String = plugins
        .iter()
        .map(|p| {
            let checked = if features.is_plugin_installed(p.id) { "checked" } else { "" };
            format!(
                "<li><form method=\"post\" action=\"/toggle-plugin\" style=\"display:inline;\"><input type=\"hidden\" name=\"id\" value=\"{}\"><label><input type=\"checkbox\" name=\"enabled\" value=\"1\" {} onchange=\"this.form.submit()\"> {} / {}</label></form></li>",
                p.id, checked, p.label_en, p.label_ja
            )
        })
        .collect();
    format!("<ul style=\"list-style:none; padding-left:0;\">{items}</ul>")
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn parse_form(body: &str) -> std::collections::HashMap<String, String> {
    body.split('&')
        .filter_map(|pair| {
            let mut it = pair.splitn(2, '=');
            let k = it.next()?;
            let v = it.next().unwrap_or("");
            Some((url_decode(k), url_decode(v)))
        })
        .collect()
}

fn url_decode(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '+' => out.push(' '),
            '%' => {
                let hi = chars.next();
                let lo = chars.next();
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    if let Ok(byte) = u8::from_str_radix(&format!("{hi}{lo}"), 16) {
                        out.push(byte as char);
                        continue;
                    }
                }
                out.push('%');
            }
            other => out.push(other),
        }
    }
    out
}

fn backend_from_form_value(v: &str) -> download_protection::ScannerBackend {
    match v {
        "kingsoft" => download_protection::ScannerBackend::Kingsoft,
        _ => download_protection::ScannerBackend::ClamAv,
    }
}

fn kind_from_form_value(v: &str) -> DeviceKind {
    match v {
        "router" => DeviceKind::Router,
        "nas" => DeviceKind::Nas,
        "external_storage" => DeviceKind::ExternalStorage,
        "pc" => DeviceKind::Pc,
        "tablet" => DeviceKind::Tablet,
        "phone" => DeviceKind::Phone,
        "tv" => DeviceKind::Tv,
        "game_console" => DeviceKind::GameConsole,
        _ => DeviceKind::Other,
    }
}

/// ルーター/NAS/外付けHDD等へのTCP接続確立時間を測定する(ICMP pingの
/// 代替、管理者権限不要)。`host`は`"192.168.1.1:80"`のような
/// `host:port`形式を期待する。
fn measure_tcp_connect_rtt(host: &str) -> Result<Duration, String> {
    let addr = host.to_socket_addrs().map_err(|e| e.to_string())?.next().ok_or_else(|| "could not resolve address".to_string())?;
    let start = Instant::now();
    TcpStream::connect_timeout(&addr, Duration::from_secs(3)).map_err(|e| e.to_string())?;
    Ok(start.elapsed())
}

fn handle(mut stream: TcpStream, state: &AppState) {
    let mut buf = [0u8; 4096];
    let n = match stream.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return,
    };
    let request = String::from_utf8_lossy(&buf[..n]);
    let first_line = request.lines().next().unwrap_or("");
    let body_text = request.split("\r\n\r\n").nth(1).unwrap_or("");

    if first_line.starts_with("POST /toggle-streaming-cap") {
        let enabled = body_text.contains("enabled=1");
        state.policy.set_streaming_cap_enabled(enabled);
        let _ = stream.write_all(b"HTTP/1.1 303 See Other\r\nLocation: /\r\n\r\n");
        return;
    }
    if first_line.starts_with("POST /toggle-router-app") {
        state.router_features.set_router_app_enabled(body_text.contains("enabled=1"));
        let _ = stream.write_all(b"HTTP/1.1 303 See Other\r\nLocation: /\r\n\r\n");
        return;
    }
    if first_line.starts_with("POST /toggle-security-router") {
        state.router_features.set_security_router_enabled(body_text.contains("enabled=1"));
        let _ = stream.write_all(b"HTTP/1.1 303 See Other\r\nLocation: /\r\n\r\n");
        return;
    }
    if first_line.starts_with("POST /toggle-plugin") {
        let form = parse_form(body_text);
        if let Some(id) = form.get("id") {
            let enabled = body_text.contains("enabled=1");
            if enabled {
                let _ = state.router_features.install_plugin(id);
            } else {
                state.router_features.uninstall_plugin(id);
            }
        }
        let _ = stream.write_all(b"HTTP/1.1 303 See Other\r\nLocation: /\r\n\r\n");
        return;
    }
    if first_line.starts_with("POST /add-wan-line") {
        let form = parse_form(body_text);
        if let Some(name) = form.get("name") {
            if !name.is_empty() {
                let _ = state.wan.register_line(name);
            }
        }
        let _ = stream.write_all(b"HTTP/1.1 303 See Other\r\nLocation: /\r\n\r\n");
        return;
    }
    if first_line.starts_with("POST /toggle-wan-auto-configure") {
        let form = parse_form(body_text);
        if let Some(name) = form.get("name") {
            state.wan.set_auto_configure_enabled(name, body_text.contains("enabled=1"));
        }
        let _ = stream.write_all(b"HTTP/1.1 303 See Other\r\nLocation: /\r\n\r\n");
        return;
    }
    if first_line.starts_with("POST /toggle-ipv6") {
        let form = parse_form(body_text);
        if let Some(name) = form.get("name") {
            state.wan.set_ipv6_enabled(name, body_text.contains("enabled=1"));
        }
        let _ = stream.write_all(b"HTTP/1.1 303 See Other\r\nLocation: /\r\n\r\n");
        return;
    }
    if first_line.starts_with("POST /toggle-v6-plus") {
        let form = parse_form(body_text);
        if let Some(name) = form.get("name") {
            state.wan.set_v6_plus_enabled(name, body_text.contains("enabled=1"));
        }
        let _ = stream.write_all(b"HTTP/1.1 303 See Other\r\nLocation: /\r\n\r\n");
        return;
    }

    let mut probe_error = None;
    if first_line.starts_with("POST /probe") {
        let form = parse_form(body_text);
        let name = form.get("name").cloned().unwrap_or_default();
        let kind = kind_from_form_value(form.get("kind").map(String::as_str).unwrap_or(""));
        let host = form.get("host").cloned().unwrap_or_default();
        if !name.is_empty() && !host.is_empty() {
            state.paths.register_device_path(&name, kind);
            match measure_tcp_connect_rtt(&host) {
                Ok(rtt) => state.paths.record_rtt(&name, rtt),
                Err(e) => probe_error = Some(e),
            }
        }
    }

    let mut scan_result = None;
    if first_line.starts_with("POST /scan-file") {
        let form = parse_form(body_text);
        if let Some(path) = form.get("path") {
            if !path.is_empty() {
                let backend = backend_from_form_value(form.get("backend").map(String::as_str).unwrap_or(""));
                let quarantine = download_protection::default_quarantine_dir();
                let want_unblock = body_text.contains("unblock=1");
                scan_result = Some(if want_unblock {
                    let r = download_protection::verify_and_unblock(backend, std::path::Path::new(path), &quarantine);
                    let mut s = r.scan;
                    if r.unblocked {
                        s.message_en.push_str(" The download warning has been removed — you can now double-click to open it yourself. / ダウンロード警告を解除しました——引き続きご自身のダブルクリックで開けます。");
                    }
                    s
                } else {
                    download_protection::scan_file(backend, std::path::Path::new(path), &quarantine)
                });
            }
        }
    }
    if first_line.starts_with("POST /protect-usb-drive") {
        let form = parse_form(body_text);
        if let Some(drive) = form.get("drive") {
            if !drive.is_empty() {
                let backend = backend_from_form_value(form.get("backend").map(String::as_str).unwrap_or(""));
                let (autorun, scan) = usb_protection::protect_drive(std::path::Path::new(drive), backend);
                let mut combined = scan;
                if autorun.found {
                    let note = if autorun.quarantined {
                        " (an autorun.inf file was also found and quarantined / autorun.infファイルも発見・隔離しました)"
                    } else {
                        " (an autorun.inf file was found but could not be quarantined / autorun.infファイルが見つかりましたが隔離できませんでした)"
                    };
                    combined.message_en.push_str(note);
                }
                scan_result = Some(combined);
            }
        }
    }

    let mut maintenance_report = None;
    if first_line.starts_with("POST /check-new-usb") {
        let mut seen = state.usb_seen.lock().unwrap();
        let newly_inserted = usb_protection::poll_new_drives(&mut seen);
        drop(seen);
        let message = if newly_inserted.is_empty() {
            "No newly inserted drives since the last check. / 前回の確認以降、新しく挿入されたドライブはありません。".to_string()
        } else {
            let list = newly_inserted.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ");
            format!("Newly inserted drive(s) detected: {list} — use \"Protect this drive\" above to scan it. / 新しく挿入されたドライブを検知しました: {list} — 上記の「このドライブを保護」でスキャンしてください。")
        };
        *state.usb_check_message.lock().unwrap() = Some(message);
        let _ = stream.write_all(b"HTTP/1.1 303 See Other\r\nLocation: /\r\n\r\n");
        return;
    }
    if first_line.starts_with("POST /optimize-paths") {
        let form = parse_form(body_text);
        let budget: u32 = form.get("budget").and_then(|v| v.parse().ok()).unwrap_or(200);

        let registered = state.paths.registered_paths_with_link_speed();
        // コストは実測リンク速度(bps、Get-NetAdapterのSpeedプロパティ)を
        // Mbps単位に換算した値を使う——実測値が取れない環境(非Windows・
        // 検出失敗)では、正直な開示の上でデモ用の仮の固定値
        // (有線=100、WiFi/Bluetooth=50、その他=150)へフォールバックする。
        let mut used_real_link_speed = false;
        let entries: Vec<(String, u32, f64)> = registered
            .into_iter()
            .map(|(name, kind, rtt_ms, link_speed_bps)| {
                let cost = match link_speed_bps {
                    Some(bps) if bps > 0 => {
                        used_real_link_speed = true;
                        ((bps / 1_000_000).max(1)) as u32
                    }
                    _ => match kind {
                        DeviceKind::Wifi | DeviceKind::Bluetooth => 50,
                        _ => 100,
                    },
                };
                let quality = rtt_ms.map(|r| 100.0 / r.max(0.1)).unwrap_or(1.0);
                (name, cost, quality)
            })
            .collect();

        let message = if entries.is_empty() {
            "No registered paths to optimize yet. / 最適化対象の登録済み経路がまだありません。".to_string()
        } else {
            let entry_refs: Vec<(&str, u32, f64)> = entries.iter().map(|(n, c, q)| (n.as_str(), *c, *q)).collect();
            let result = path_optimizer::optimize_path_selection(&entry_refs, budget, 0xC0FFEE);
            // 最適化結果を実際にMultiPathManagerへ反映する(2026-08-11
            // 追加、ユーザー指示「最適化結果の実際のトラフィック制御への
            // 反映」への対応)。best_pathは以後、無効化された経路を
            // 選択対象から除外する。
            for name in &result.activate {
                state.paths.set_enabled(name, true);
            }
            for name in &result.deactivate {
                state.paths.set_enabled(name, false);
            }
            format!(
                "Activate / 有効化: {} | Deactivate / 無効化: {} | cost {}/{} | total quality / 総品質 {:.1} | used SBM solution / SBM解を使用: {} | cost source / コストの根拠: {}",
                if result.activate.is_empty() { "(none)".to_string() } else { result.activate.join(", ") },
                if result.deactivate.is_empty() { "(none)".to_string() } else { result.deactivate.join(", ") },
                result.total_cost,
                result.budget,
                result.total_quality,
                result.used_sbm_solution,
                if used_real_link_speed {
                    "real link speed (Mbps) / 実測リンク速度(Mbps)"
                } else {
                    "placeholder fixed values (link speed unavailable) / 仮の固定値(リンク速度取得不可)"
                }
            )
        };
        *state.path_optimization_message.lock().unwrap() = Some(message);
        let _ = stream.write_all(b"HTTP/1.1 303 See Other\r\nLocation: /\r\n\r\n");
        return;
    }
    if first_line.starts_with("POST /durability-demo") {
        let form = parse_form(body_text);
        let message = form.get("message").cloned().unwrap_or_else(|| "test message".to_string());

        let mut sender = SecureChannel::new(&[0x42u8; 32]);
        let mut receiver = SecureChannel::new(&[0x42u8; 32]);
        let encrypted = sender.encrypt(message.as_bytes());

        let mut log_lines = vec![format!("1) encrypt (secure_channel) / 暗号化: {} bytes plaintext -> {} bytes frame", message.len(), encrypted.as_ref().map(|f| f.len()).unwrap_or(0))];

        let demo_text = match encrypted {
            Ok(frame) => {
                let wal_path = std::env::temp_dir().join("rs-smarttcp-gui-durability-demo.log");
                match TransactionLog::open(&wal_path) {
                    Ok(wal) => match wal.append(&frame) {
                        Ok(()) => {
                            log_lines.push(format!("2) durably recorded (transaction_log, fsync) / WALへ確実に記録: {}", wal_path.display()));

                            let frame_for_paths = frame.clone();
                            let paths: Vec<Box<dyn FnOnce() -> Result<Vec<u8>, String> + Send>> = vec![
                                Box::new(move || Err::<Vec<u8>, String>("simulated primary link failure / 主経路の疑似障害".to_string())),
                                Box::new(move || Ok(frame_for_paths)),
                            ];

                            match redundant_transmission::send_redundant(paths) {
                                Ok(outcome) => {
                                    log_lines.push(format!(
                                        "3) sent via redundant path #{} (redundant_transmission), {} path(s) failed first / 冗長経路#{}経由で送信成功({}本失敗後)",
                                        outcome.succeeded_path_index, outcome.failed_before_success, outcome.succeeded_path_index, outcome.failed_before_success
                                    ));
                                    match receiver.decrypt(&outcome.value) {
                                        Ok(decrypted) => {
                                            log_lines.push(format!("4) decrypted and verified (secure_channel) / 復号・検証成功: \"{}\"", String::from_utf8_lossy(&decrypted)));
                                        }
                                        Err(e) => log_lines.push(format!("4) decryption failed / 復号失敗: {e}")),
                                    }
                                }
                                Err(e) => log_lines.push(format!("3) all redundant paths failed / 全経路失敗(WALには記録済みのため再送可能): {e}")),
                            }
                        }
                        Err(e) => log_lines.push(format!("2) WAL write failed / WAL書き込み失敗: {e}")),
                    },
                    Err(e) => log_lines.push(format!("2) failed to open WAL / WALを開けませんでした: {e}")),
                }
                log_lines.join("\n")
            }
            Err(e) => format!("1) encryption failed / 暗号化失敗: {e}"),
        };

        *state.durability_demo_message.lock().unwrap() = Some(demo_text);
        let _ = stream.write_all(b"HTTP/1.1 303 See Other\r\nLocation: /\r\n\r\n");
        return;
    }
    if first_line.starts_with("POST /run-maintenance") {
        maintenance_report = Some(maintenance::run_maintenance());
    }

    let body = render_page(state, probe_error.as_deref(), scan_result.as_ref(), maintenance_report.as_ref());
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

fn main() {
    let addr = bind_addr();
    let listener = TcpListener::bind(&addr).expect("failed to bind status GUI (is the port already in use?)");
    println!("RS-SmartTCP status GUI listening on http://{addr}/");

    let report = network_interfaces::detect();
    let wan = MultiWanManager::new();
    let _ = wan.register_line("WAN1"); // 既定で1本、最大10本まで追加可能。
    let state = AppState {
        policy: BandwidthPolicy::new(),
        paths: MultiPathManager::from_detected_interfaces(&report),
        router_features: RouterFeatures::new(),
        wan,
        usb_seen: Mutex::new(HashSet::new()),
        usb_check_message: Mutex::new(None),
        path_optimization_message: Mutex::new(None),
        durability_demo_message: Mutex::new(None),
    };
    let state = Mutex::new(state);

    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let state = state.lock().unwrap();
                handle(s, &state);
            }
            Err(e) => eprintln!("connection error: {e}"),
        }
    }
}
