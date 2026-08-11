//! 動作確認用の簡易GUI(ステータスページ、2026-08-11新設)。
//!
//! ユーザー指示「機能確認の為のGUI化と、今何がつながっているかの確認
//! 機能も付けて」への対応。標準ライブラリのみ(`std::net`)で実装した
//! 最小限のHTTPサーバー——このクレートの既存方針「外部依存クレート
//! 無し」を保つため、RPoem等のWebフレームワークは意図的に使わない。
//!
//! 実行方法: `cargo run --example status_gui`(既定`http://127.0.0.1:7878/`、
//! `RS_SMARTTCP_GUI_BIND`環境変数で上書き可)。
//!
//! **正直な開示**: これは最小限の単一リクエスト処理サーバーであり、
//! 同時多接続・HTTPS・堅牢なエラーハンドリングは無い(動作確認用の
//! 簡易ツールとしての位置づけ)。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

use rs_smarttcp::bandwidth_policy::BandwidthPolicy;
use rs_smarttcp::network_interfaces;

fn bind_addr() -> String {
    std::env::var("RS_SMARTTCP_GUI_BIND").unwrap_or_else(|_| "127.0.0.1:7878".to_string())
}

fn render_page(policy: &BandwidthPolicy) -> String {
    let report = network_interfaces::detect();
    let wired = report.wired_connected_count();
    let wifi = if report.wifi_connected() { "Connected / 接続中" } else { "Not connected / 未接続" };
    let checked = if policy.is_streaming_cap_enabled() { "checked" } else { "" };

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

    format!(
        r#"<!doctype html>
<html><head><meta charset="utf-8"><title>RS-SmartTCP status</title></head>
<body style="font-family: sans-serif; max-width: 640px; margin: 40px auto;">
<h1>RS-SmartTCP — Connection status / 接続状況</h1>
<p>Wired Ethernet connected / 有線LAN接続本数: <strong>{wired}</strong> (max 4 supported / 最大4本まで対応)</p>
<p>Wi-Fi / WiFi: <strong>{wifi}</strong></p>
<table border="1" cellpadding="6" style="border-collapse: collapse;">
<tr><th>Interface / インターフェース</th><th>Kind / 種別</th><th>Status / 状態</th></tr>
{rows}
</table>
<form method="post" action="/toggle-streaming-cap" style="margin-top: 20px;">
<label>
<input type="checkbox" name="enabled" value="1" {checked} onchange="this.form.submit()">
Fix speed to 10Mbps for streaming (YouTube / U-NEXT / Qobuz etc.) to improve audio quality? /
音質向上のため、動画・音楽ストリーミング(YouTube・U-NEXT・Qobuz等)利用時の通信速度を10Mbpsに固定しますか？
</label>
</form>
<p style="color:#666; font-size: 0.9em;">Other traffic (regular websites, SFTP, Claude and other AI/chat tools) always runs at full speed. /
それ以外の通信(通常のWebサイト・SFTP・ClaudeなどのAI・チャットツール等)は常に最高速度で動作します。</p>
<p style="color:#999; font-size: 0.8em;">Honest disclosure: this does not sum the bandwidth of multiple links into one faster connection (true link aggregation requires OS/NIC teaming support). It picks the best-performing path and fails over automatically. / 正直な開示: 複数回線の速度を合算する機能ではありません(本物のリンクアグリゲーションにはOS/NICのチーミング機能が必要です)。最良経路の選択と自動フェイルオーバーを行います。</p>
</body></html>"#
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn handle(mut stream: TcpStream, policy: &BandwidthPolicy) {
    let mut buf = [0u8; 4096];
    let n = match stream.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return,
    };
    let request = String::from_utf8_lossy(&buf[..n]);
    let first_line = request.lines().next().unwrap_or("");

    let body;
    let status_line;
    if first_line.starts_with("POST /toggle-streaming-cap") {
        let enabled = request.contains("enabled=1");
        policy.set_streaming_cap_enabled(enabled);
        status_line = "HTTP/1.1 303 See Other\r\nLocation: /\r\n\r\n";
        let _ = stream.write_all(status_line.as_bytes());
        return;
    } else {
        body = render_page(policy);
        status_line = "HTTP/1.1 200 OK\r\n";
    }

    let response = format!(
        "{status_line}Content-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

fn main() {
    let addr = bind_addr();
    let listener = TcpListener::bind(&addr).expect("failed to bind status GUI (is the port already in use?)");
    println!("RS-SmartTCP status GUI listening on http://{addr}/");
    let policy = Arc::new(BandwidthPolicy::new());

    for stream in listener.incoming() {
        match stream {
            Ok(s) => handle(s, &policy),
            Err(e) => eprintln!("connection error: {e}"),
        }
    }
}
