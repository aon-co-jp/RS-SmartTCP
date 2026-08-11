//! 複数経路への同時多重送信(2026-08-11新設、ユーザー指示「4層4重の
//! 通信…オンライン証券などのネット上のDATAを紛失しない設計思想」への
//! 対応、スコープ確認の結果「複数WAN/LAN経路で同一データを最大4重送信し、
//! どれか1つ届けばACKとする冗長化」と定義)。
//!
//! ## 正直な開示(最重要)
//!
//! - **本モジュールが提供するのは、渡された送信関数を最大4本の経路へ
//!   並行に呼び出し、最初に成功した結果を採用するオーケストレーション
//!   のみ**——実際のソケット送受信・暗号化・ネットワークI/Oは呼び出し
//!   側が渡す`Fn() -> Result<T, E>`クロージャの中身に依存する(本
//!   クレートは`std::net`より上の抽象化に留め、TCP/UDP/QUIC等の具体的な
//!   トランスポートを決め打ちしない、既存の`multi_path`/`multi_wan`と
//!   同じ設計方針)。
//! - **「データを紛失しない」ことの保証範囲**: 4本の経路すべてが同時に
//!   失敗しない限り、送信自体は少なくとも1本成功する可能性を高める
//!   (可用性の向上)。ただし、これは**永続化(ディスクへの確実な書き込み)
//!   の保証ではない**——真にデータを失わない設計にするには、送信前に
//!   [`crate::transaction_log`]のようなWAL(Write-Ahead Log)へ先に
//!   確実に書き込んでおき、送信に全経路失敗した場合でも再送できるように
//!   する必要がある(この2モジュールを組み合わせて使うことを想定した
//!   設計、詳細は[`crate::transaction_log`]のモジュールdoc参照)。
//! - **重複排除は呼び出し側の責務**: 4本の経路のうち複数が成功した場合
//!   (例: 3本目に到達確認できる前に4本目も届いてしまった)、受信側で
//!   同じデータが複数回届く可能性がある——本モジュールは冪等性キー等の
//!   重複排除機構を持たない(`RS-JSON`/`open-web-server`等、実際の
//!   アプリケーション層で冪等キーを設計すべき領域だと判断し、通信層である
//!   本クレートのスコープ外とした)。

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// 同時送信を試みる経路の最大本数(ユーザー指示「4層4重」に基づく)。
pub const MAX_REDUNDANT_PATHS: usize = 4;

#[derive(Debug, Clone)]
pub struct RedundantSendOutcome<T> {
    /// 実際に成功した経路のインデックス(`paths`引数内の位置)。
    pub succeeded_path_index: usize,
    pub value: T,
    /// 成功するまでに失敗した経路の本数(0なら1本目で即成功)。
    pub failed_before_success: usize,
}

/// 渡された最大[`MAX_REDUNDANT_PATHS`]本の送信クロージャを並行に実行し、
/// 最初に成功した結果を返す。全経路が失敗した場合は最後に観測した
/// エラーを返す(呼び出し側が原因を確認できるよう、黙って握りつぶさない)。
///
/// `paths`が[`MAX_REDUNDANT_PATHS`]本を超える場合はエラーを返す
/// (無制限の多重化は「4重」という設計意図から外れるため)。
pub fn send_redundant<T, E, F>(paths: Vec<F>) -> Result<RedundantSendOutcome<T>, String>
where
    T: Send + 'static,
    E: Send + std::fmt::Display + 'static,
    F: FnOnce() -> Result<T, E> + Send + 'static,
{
    if paths.is_empty() {
        return Err("no paths provided / 経路が1本も指定されていません".to_string());
    }
    if paths.len() > MAX_REDUNDANT_PATHS {
        return Err(format!(
            "cannot use more than {MAX_REDUNDANT_PATHS} redundant paths / 冗長経路は最大{MAX_REDUNDANT_PATHS}本までです"
        ));
    }

    let (tx, rx) = mpsc::channel();
    let total = paths.len();
    for (idx, path_fn) in paths.into_iter().enumerate() {
        let tx = tx.clone();
        thread::spawn(move || {
            let result = path_fn();
            // 送信側が既に閉じている(他の経路が先に成功しチャンネルを
            // 抜けた)場合のsend失敗は無視してよい——結果を待つ側は
            // 既に用が済んでいる。
            let _ = tx.send((idx, result.map_err(|e| e.to_string())));
        });
    }
    drop(tx);

    let mut failed = 0usize;
    let mut last_err: Option<String> = None;
    for _ in 0..total {
        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok((idx, Ok(value))) => {
                return Ok(RedundantSendOutcome { succeeded_path_index: idx, value, failed_before_success: failed });
            }
            Ok((_, Err(e))) => {
                failed += 1;
                last_err = Some(e);
            }
            Err(_) => break,
        }
    }

    Err(last_err.unwrap_or_else(|| "all redundant paths failed or timed out / 全ての冗長経路が失敗またはタイムアウトしました".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn returns_the_first_successful_path_even_if_others_would_fail() {
        // send_redundant returns as soon as the first success arrives, so it
        // does not wait for slower paths — assert only on the outcome, not on
        // how many paths happened to finish before the winner was picked
        // (asserting the latter would be a data race on this atomic counter).
        let attempts = Arc::new(AtomicUsize::new(0));
        let a1 = Arc::clone(&attempts);
        let a2 = Arc::clone(&attempts);
        let a3 = Arc::clone(&attempts);

        let paths: Vec<Box<dyn FnOnce() -> Result<&'static str, String> + Send>> = vec![
            Box::new(move || {
                a1.fetch_add(1, Ordering::SeqCst);
                Err("path1 down".to_string())
            }),
            Box::new(move || {
                a2.fetch_add(1, Ordering::SeqCst);
                Ok("delivered via path2")
            }),
            Box::new(move || {
                a3.fetch_add(1, Ordering::SeqCst);
                Ok("delivered via path3")
            }),
        ];

        let outcome = send_redundant(paths).expect("at least one path must succeed");
        assert!(outcome.value.starts_with("delivered"));
    }

    #[test]
    fn returns_error_when_all_paths_fail() {
        let paths: Vec<Box<dyn FnOnce() -> Result<(), String> + Send>> =
            vec![Box::new(|| Err("down1".to_string())), Box::new(|| Err("down2".to_string()))];
        assert!(send_redundant(paths).is_err());
    }

    #[test]
    fn rejects_more_than_max_redundant_paths() {
        let paths: Vec<Box<dyn FnOnce() -> Result<(), String> + Send>> =
            (0..MAX_REDUNDANT_PATHS + 1).map(|_| Box::new(|| Ok(())) as Box<dyn FnOnce() -> Result<(), String> + Send>).collect();
        assert!(send_redundant(paths).is_err());
    }

    #[test]
    fn rejects_empty_path_list() {
        let paths: Vec<Box<dyn FnOnce() -> Result<(), String> + Send>> = vec![];
        assert!(send_redundant(paths).is_err());
    }
}
