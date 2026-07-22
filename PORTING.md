# PORTING.md — お引越し可能ファイル

他のプロジェクトへそのまま(または軽微な変更で)移植できる実装パターン
一覧。

## SRTT/RTTVAR(RFC 6298/9002準拠のEWMA)によるネットワーク品質推定
(`src/lib.rs`)

TCP(RFC 6298)・QUIC(RFC 9002)が実際に使うJacobson/Karels EWMA
アルゴリズムをそのまま流用した、RTT/ジッター推定の最小実装。固定
ウィンドウ+標準偏差方式(全サンプルをメモリに保持し毎回平均・分散を
計算し直す)より、サンプル1件ごとにO(1)の更新のみで済む。ネットワーク
品質に応じた適応制御が必要な、あらゆるRust製ネットワーククライアント
/サーバーへそのまま移植可能。

```rust
const ALPHA: f64 = 1.0 / 8.0; // RFC 6298と同じ重み
const BETA: f64 = 1.0 / 4.0;

fn update(prev: Option<(f64, f64)>, r: f64) -> (f64, f64) {
    match prev {
        None => (r, r / 2.0), // RFC 6298 2.2節: 初回サンプルの特別扱い
        Some((srtt, rttvar)) => {
            let rttvar = (1.0 - BETA) * rttvar + BETA * (srtt - r).abs();
            let srtt = (1.0 - ALPHA) * srtt + ALPHA * r;
            (srtt, rttvar)
        }
    }
}
```

## Fast/Slow 2段階適応方針パターン(`src/lib.rs`)

「ネットワーク品質の判定結果を`enum`で表し、呼び出し側のパラメータ
(リトライ間隔・タイムアウト・バッチサイズ等)を2段階で切り替える」
という設計は、ネットワーク品質に限らず、あらゆる「観測に基づいて
挙動を2モードで切り替えたい」場面(負荷に応じたスロットリング、
エラー率に応じたサーキットブレーカー等)に応用できる汎用パターン。

```rust
pub enum AdaptiveMode { Fast, Slow }

impl AdaptivePolicy {
    pub fn mode(&self) -> AdaptiveMode {
        if self.monitor.is_photonic_class() { AdaptiveMode::Fast } else { AdaptiveMode::Slow }
    }
}
```

## 未実装拡張点を安全にフォールバックさせるパターン(移植元:
`open-web-server-wire::accel`、本リポジトリと同日に新設)

「将来対応予定のハードウェア/バックエンドを`enum`の選択肢として先に
定義し、未実装のものが選ばれても panic せず既定の実装へ安全に
フォールバックしつつ`tracing::warn!`で可視化する」という設計は、
本クレート自体の`AccelBackend`(GPU/NPU未実装)と同じ考え方——
API形状を将来のハードウェア/機能に合わせて先取りしておき、実装が
追いつくまでは安全側の代替で動かす、という汎用パターン。
