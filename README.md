# RS-SmartTCP

**開発開始日: 2026-07-23**(このリポジトリのGitHub作成日)

> 📌 **v0.2.0(2026-08-11)**: 当初のRTT/ジッター適応制御に加え、
> 有線LAN・WiFi・Bluetooth(各最大10チャンネル)+複数WAN回線(最大10本、
> IPv4/IPv6/v6プラス対応)の管理、ルーター/セキュリティルーター機能+
> 既知プラグイン、ダウンロードファイルのウイルススキャン(ClamAV/
> KINGSOFT)+自動実行防止、USBリムーバブルドライブ保護(autorun.inf
> 無害化)を追加。詳細は[CLAUDE.md](CLAUDE.md)のHANDOFF参照。

IOWN/APN(NTTのオールフォトニクス・ネットワーク、光電融合)のような
超低遅延・ジッター無し回線と、Smart-TCP(AI生成通信プロトコル、
fast/slowモデルによる判断構造)の良いとこ取りハイブリッド適応制御。
実測RTT・ジッターに基づき、TCP(RFC 6298)/QUIC(RFC 9002)と同じ
SRTT/RTTVAR(Jacobson/Karels EWMA)アルゴリズムでネットワーク品質を
分類し、リトライ間隔等を2段階(Fast/Slow)で切り替える。

## これは何か

- **`NetworkQualityMonitor`**: RTTサンプルを記録し、SRTT(平滑化RTT)・
  RTTVAR(RTT変動)をTCP/QUICと全く同じEWMAアルゴリズムで追跡する。
  両方が閾値未満なら「photonic-class」(IOWN/APNのような光ネットワーク
  級)、そうでなければ「standard-class」と判定する。
- **`AdaptivePolicy`**: 判定結果に応じてリトライ待機時間等を切り替える
  (Fast=光ネットワーク級向けの積極的な設定、Slow=通常インターネット
  向けの保守的な設定)。

## 正直な開示・命名の経緯

**本クレートは、arXiv 2512.00491("Agentic AI-based Autonomous and
Adaptive TCP Protocol"、"Smart-TCP")のプロトコルそのものの実装では
ない。** 訓練済み機械学習モデルは使わず、「fast/slowモデル」という
設計思想を、TCP/QUICが実際に使うSRTT/RTTVAR EWMAに基づく決定論的な
2値判定として実装したもの。`RS-SmartTCP`という名前は「Smart-TCPに
着想を得た、このエコシステム独自の実装」であることを示す
(既存の`RS-`接頭辞の命名規則に準拠)。論文の同名プロトコルと混同
しないこと。

同様に、IOWN/APN自体はNTTが構築する物理telecom基盤(光電融合スイッチ・
光ファイバー回線)であり、本クレートが「実装」できる対象ではない
——実際に行っているのは「そのような回線が来た時にソフトウェア層が
足を引っ張らない」設計のみ。

## 使用例

```rust
use rs_smarttcp::{AdaptivePolicy, NetworkQualityMonitor};
use std::time::Duration;

let policy = AdaptivePolicy::new(NetworkQualityMonitor::new());

// 実測RTTを記録していく(呼び出し側が実際に計測した値を渡す)
policy.monitor().record_rtt(Duration::from_millis(17));

// 判定に応じたリトライ間隔を取得
let backoff = policy.retry_backoff();
```

## v0.2.0で追加した機能一覧(2026-08-11)

- **`network_interfaces`**: 有線LAN・WiFi・Bluetoothの接続状況を
  `Get-NetAdapter`(PowerShell、ロケール非依存の技術定数で判定)経由で
  検出。
- **`multi_path`**: 有線LAN最大10本+WiFi最大10チャンネル+Bluetooth
  最大10チャンネルの中から最良RTTの経路を選び、自動フェイルオーバーする
  (`MultiPathManager`)。ルーター・NAS・外付けHDD・PC・タブレット・
  スマホ・TV・ゲーム機のラベル付けにも対応。**正直な開示**: 本物の
  帯域合算リンクアグリゲーションではない(OS/NICチーミングが必要)。
- **`multi_wan`**: 名前付きWAN回線を最大10本管理(`MultiWanManager`)。
- **`wan_config`**: IPv4/IPv6/v6プラス(MAP-E)+WAN自動設定フラグ
  (`WanConfig`)。**正直な開示**: 実際のDHCPv6-PD交渉・トンネル確立は
  OS/ルーター機器側が行う、設定意図フラグのみ。
- **`bandwidth_policy`**: YouTube/U-NEXT/Qobuz等のストリーミング時のみ
  10Mbps固定、それ以外(SFTP・AIチャットツール等)は常に最高速度。
- **`router_features`**: ルーターアプリ/セキュリティルーター機能の
  チェックボックス+既知プラグイン(ポート転送・QoS・DHCP、広告ブロック・
  DNSフィルタ・ペアレンタルコントロール、TLS検査/AI侵入検知は設定
  フラグのみで未実装)。**正直な開示**: 任意の外部コード実行機構ではない。
- **`download_protection`**: ダウンロードファイルをClamAV(オープン
  ソース)またはKINGSOFT Internet Security(無料版)で実際にスキャン。
  脅威は削除ではなく隔離フォルダへ移動(可逆的)。Windows「Mark of the
  Web」による自動実行防止。Windows Security Center未登録時のセキュリティ
  ソフト導入推奨メッセージ(日英)。
- **`usb_protection`**: リムーバブルドライブ検出+`autorun.inf`無害化
  (リネーム、削除ではない)+ドライブスキャン。

`examples/status_gui.rs`(`cargo run --example status_gui`)で全機能を
実際にブラウザから確認できる。

## このエコシステムでの利用箇所

[`open-web-server-wire`](https://github.com/aon-co-jp/open-web-server)
からpath依存として利用される(`Rust-JSON`が`aruaru-db`等から利用される
のと同じ「独立リポジトリとして切り出し、必要な場所からpath依存する」
パターン)。

## ビルド・テスト

```bash
cargo test
```

外部依存クレート無し(標準ライブラリのみ)。

## ライセンス

Apache-2.0
