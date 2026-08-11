//! 動作確認用の簡易GUI(ステータスページ、2026-08-11新設)。
//!
//! ユーザー指示「機能確認の為のGUI化と、今何がつながっているかの確認
//! 機能も付けて」+「ルーターと外付けHDDやNASなどに複数LANケーブル
//! 1本から最大4本＋WiFiも追加可能にして対応して」への対応。標準
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

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rs_smarttcp::bandwidth_policy::BandwidthPolicy;
use rs_smarttcp::multi_path::{DeviceKind, MultiPathManager};
use rs_smarttcp::network_interfaces;
use rs_smarttcp::router_features::{RouterFeatures, ROUTER_APP_PLUGINS, SECURITY_ROUTER_PLUGINS};
use rs_smarttcp::wan_config::WanConfig;

fn bind_addr() -> String {
    std::env::var("RS_SMARTTCP_GUI_BIND").unwrap_or_else(|_| "127.0.0.1:7878".to_string())
}

struct AppState {
    policy: BandwidthPolicy,
    paths: MultiPathManager,
    router_features: RouterFeatures,
    wan: WanConfig,
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

fn render_page(state: &AppState, probe_error: Option<&str>) -> String {
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
        .registered_paths()
        .into_iter()
        .map(|(name, kind, rtt_ms)| {
            let is_best = best.as_deref() == Some(name.as_str());
            format!(
                "<tr{}><td>{}</td><td>{}</td><td>{}</td></tr>",
                if is_best { " style=\"font-weight:bold;background:#eef8ee;\"" } else { "" },
                html_escape(&name),
                device_kind_label(kind),
                rtt_ms.map(|v| format!("{v:.1} ms{}", if is_best { " (best / 最良経路)" } else { "" })).unwrap_or_else(|| "no data / 未測定".to_string())
            )
        })
        .collect();

    let error_html = probe_error
        .map(|e| format!("<p style=\"color:#c33;\">Probe failed / 疎通確認に失敗しました: {}</p>", html_escape(e)))
        .unwrap_or_default();

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

    let wan_summary = state.wan.connection_summary();
    let wan_auto_checked = if state.wan.is_auto_configure_enabled() { "checked" } else { "" };
    let ipv6_checked = if state.wan.is_ipv6_enabled() { "checked" } else { "" };
    let v6_plus_checked = if state.wan.is_v6_plus_enabled() { "checked" } else { "" };

    format!(
        r#"<!doctype html>
<html><head><meta charset="utf-8"><title>RS-SmartTCP status</title></head>
<body style="font-family: sans-serif; max-width: 720px; margin: 40px auto;">
<h1>RS-SmartTCP — Connection status / 接続状況</h1>
<p>Wired Ethernet connected / 有線LAN接続本数: <strong>{wired}</strong> (max 4 supported / 最大4本まで対応)</p>
<p>Wi-Fi connected / WiFi接続本数: <strong>{wifi_count}</strong> (multiple adapters supported / 複数枚対応)</p>
<p>Bluetooth connected / Bluetooth接続本数: <strong>{bt_count}</strong> (multiple adapters supported / 複数対応)</p>
<table border="1" cellpadding="6" style="border-collapse: collapse;">
<tr><th>Interface / インターフェース</th><th>Kind / 種別</th><th>Status / 状態</th></tr>
{rows}
</table>

<h2>Router / NAS / External HDD paths / 経路一覧</h2>
<p style="font-size:0.85em; color:#666;">Add your router, NAS, or external HDD's address below to measure and compare its response time (up to 4 wired + Wi-Fi). / 下のフォームからルーター・NAS・外付けHDDのアドレスを追加すると、応答時間を測定・比較できます(有線最大4本+WiFi)。</p>
<table border="1" cellpadding="6" style="border-collapse: collapse;">
<tr><th>Name / 名前</th><th>Kind / 種別</th><th>Response time / 応答時間</th></tr>
{device_rows}
</table>
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

<h2>WAN connection / WAN接続設定</h2>
<p>Current mode / 現在の方式: <strong>{wan_summary}</strong></p>
<form method="post" action="/toggle-wan-auto-configure">
<label><input type="checkbox" name="enabled" value="1" {wan_auto_checked} onchange="this.form.submit()"> Auto-configure WAN connection / WANからの接続を自動設定</label>
</form>
<form method="post" action="/toggle-ipv6">
<label><input type="checkbox" name="enabled" value="1" {ipv6_checked} onchange="this.form.submit()"> Use IPv6 / IPv6を使用する</label>
</form>
<form method="post" action="/toggle-v6-plus">
<label><input type="checkbox" name="enabled" value="1" {v6_plus_checked} onchange="this.form.submit()"> IPv6 v6 Plus (MAP-E) / IPv6 v6プラス(MAP-E)</label>
</form>
<p style="font-size:0.85em; color:#666;">You can use IPv6 without v6 Plus (e.g. native/PPPoE IPv6) by leaving the v6 Plus box unchecked. / v6プラスのチェックを外したままでも、IPv6自体(ネイティブ/PPPoE方式等)は利用できます。</p>
<p style="color:#999; font-size: 0.8em;">Honest disclosure: these are configuration-intent flags only — actual WAN negotiation (DHCPv6-PD, MAP-E parameter retrieval, tunnel setup) is performed by your OS/router firmware, not by this library. / 正直な開示: これらは設定意図を表すフラグに過ぎません——実際のWAN接続確立(DHCPv6-PD交渉・MAP-Eパラメータ取得・トンネル設定)はOS/ルーター機器側が行い、このライブラリ自体は行いません。</p>
<p style="color:#666; font-size: 0.9em;">Other traffic (regular websites, SFTP, Claude and other AI/chat tools) always runs at full speed. /
それ以外の通信(通常のWebサイト・SFTP・ClaudeなどのAI・チャットツール等)は常に最高速度で動作します。</p>
<p style="color:#999; font-size: 0.8em;">Honest disclosure: this does not sum the bandwidth of multiple links into one faster connection (true link aggregation requires OS/NIC teaming support). It picks the best-performing path and fails over automatically. Response time is measured via TCP connect time, not ICMP ping. / 正直な開示: 複数回線の速度を合算する機能ではありません(本物のリンクアグリゲーションにはOS/NICのチーミング機能が必要です)。最良経路の選択と自動フェイルオーバーを行います。応答時間はICMP pingではなくTCP接続確立時間で測定しています。</p>
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
    if first_line.starts_with("POST /toggle-wan-auto-configure") {
        state.wan.set_auto_configure_enabled(body_text.contains("enabled=1"));
        let _ = stream.write_all(b"HTTP/1.1 303 See Other\r\nLocation: /\r\n\r\n");
        return;
    }
    if first_line.starts_with("POST /toggle-ipv6") {
        state.wan.set_ipv6_enabled(body_text.contains("enabled=1"));
        let _ = stream.write_all(b"HTTP/1.1 303 See Other\r\nLocation: /\r\n\r\n");
        return;
    }
    if first_line.starts_with("POST /toggle-v6-plus") {
        state.wan.set_v6_plus_enabled(body_text.contains("enabled=1"));
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

    let body = render_page(state, probe_error.as_deref());
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
    let state = AppState {
        policy: BandwidthPolicy::new(),
        paths: MultiPathManager::from_detected_interfaces(&report),
        router_features: RouterFeatures::new(),
        wan: WanConfig::new(),
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
