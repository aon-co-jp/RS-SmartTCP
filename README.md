# RS-SmartTCP

**開発開始日: 2026-07-23**(このリポジトリのGitHub作成日)

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
