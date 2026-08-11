//! Write-Ahead Log(WAL)による最小限のACID互換トランザクションログ
//! (2026-08-11新設、ユーザー指示「ACID互換…オンライン証券などのネット
//! 上のDATAを紛失しない設計思想」への対応、スコープ確認の結果
//! 「RS-SmartTCP自体がACIDトランザクションを実装する」と確認)。
//!
//! ## 正直な開示(最重要・スコープの明確化)
//!
//! **本モジュールは、`open-raid-z`(ZFS互換のRust実装)のような本格的な
//! ファイルシステム/ストレージエンジンではない。** 提供するのは
//! 「1レコードの追記が必ず全体成功するか全く反映されないか」という
//! 単純な追記専用WALであり、以下の4特性それぞれについて、何を・どう
//! 満たすかを正直に区切って明記する:
//!
//! - **Atomicity(原子性)**: 1レコード = `[4バイト長][4バイトCRC32]
//!   [ペイロード]`という固定フォーマットで書き込む。書き込み途中で
//!   プロセスが落ちた場合、次回読み込み時に長さ/CRC不一致として検出し、
//!   その不完全なレコードを「無かったもの」として無視する(部分書き込み
//!   が「読めてしまう」ことを防ぐ)。
//! - **Consistency(一貫性)**: 各レコードのCRC32チェックサムで、書き込み
//!   後の破損(ディスク不良等)を検出する。**本モジュールは業務データの
//!   意味的な整合性制約(例: 残高がマイナスにならない)までは検証しない**
//!   ——それは呼び出し側アプリケーションの責務。
//! - **Isolation(分離性)**: `Mutex`で書き込みを完全に直列化する。複数
//!   スレッドが同時に`append`を呼んでも、レコード同士が混ざることは
//!   ない。**複数の追記をまたぐ「複数ステップのトランザクション」の
//!   分離レベル(スナップショット分離等)は実装していない**——
//!   1回の`append`が最小のトランザクション単位。
//! - **Durability(永続性)**: 各`append`の最後に`File::sync_data()`
//!   (`fsync`相当)を呼び、OSページキャッシュに留まらずディスクへ実際に
//!   書き込まれたことを確認してから呼び出し元へ`Ok`を返す。
//!   **RAID/複製・地理冗長化は行わない**——それはこのエコシステムでは
//!   `open-raid-z`の責務であり、本モジュールは「1台のディスク上の
//!   1ファイルへの追記」を確実にすることのみを保証する。
//!
//! ## 他リポジトリとの役割分担(重複実装を避ける)
//!
//! - **`open-raid-z`**: ZFS互換のスナップショット・チェックサム
//!   スクラブ・複数ディスクにまたがるRAID冗長化(本モジュールが保証
//!   しない「ディスク自体の破損・紛失」からの保護)。
//! - **`aruaru-db`/PostgreSQL**: 複数レコード・複数テーブルにまたがる
//!   本格的なSQLトランザクション(JOIN・複雑な整合性制約)。本モジュール
//!   はSQLエンジンの代替ではない。
//! - **本モジュール(`RS-SmartTCP::transaction_log`)**: 通信層で
//!   「送信を試みる前に、何を送ろうとしたかをまずローカルに確実に記録
//!   しておく」ためのWAL——[`crate::redundant_transmission`]と組み合わせ、
//!   「WALへ書き込み(Durability確保)→複数経路へ冗長送信→成功したら
//!   ACK」というオンライン証券のような「送信を試みたこと自体を絶対に
//!   忘れない」設計の土台を提供する。

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// CRC32(IEEE 802.3多項式)。外部crates.io依存を増やさないための
/// 自前実装(標準テーブル方式、`zlib`/`png`等が使うのと同じ多項式)。
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

pub struct TransactionLog {
    path: PathBuf,
    file: Mutex<File>,
}

impl TransactionLog {
    /// 指定パスのWALファイルを開く(無ければ新規作成、あれば追記継続)。
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = OpenOptions::new().create(true).read(true).append(true).open(path).map_err(|e| e.to_string())?;
        Ok(Self { path: path.to_path_buf(), file: Mutex::new(file) })
    }

    /// 1レコードを追記する。`[4バイト長(LE)][4バイトCRC32(LE)]
    /// [ペイロード]`のフォーマットで書き込み、`sync_data()`が成功して
    /// 初めて`Ok`を返す(Durability、冒頭のモジュールdoc参照)。
    pub fn append(&self, payload: &[u8]) -> Result<(), String> {
        let mut file = self.file.lock().unwrap();
        let len = payload.len() as u32;
        let crc = crc32(payload);
        let mut record = Vec::with_capacity(8 + payload.len());
        record.extend_from_slice(&len.to_le_bytes());
        record.extend_from_slice(&crc.to_le_bytes());
        record.extend_from_slice(payload);

        file.write_all(&record).map_err(|e| e.to_string())?;
        file.sync_data().map_err(|e| format!("fsync failed (durability not guaranteed): {e}"))?;
        Ok(())
    }

    /// 現在ファイルに書かれている、完全かつチェックサムが一致する
    /// レコードのみを順に返す(冒頭のAtomicity節参照: 途中で壊れた/
    /// 不完全なレコードは黙って無視し、それ以降は読み取りを打ち切る)。
    pub fn read_all_valid_records(&self) -> Result<Vec<Vec<u8>>, String> {
        let mut file = File::open(&self.path).map_err(|e| e.to_string())?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).map_err(|e| e.to_string())?;

        let mut records = Vec::new();
        let mut offset = 0usize;
        while offset + 8 <= buf.len() {
            let len = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap()) as usize;
            let stored_crc = u32::from_le_bytes(buf[offset + 4..offset + 8].try_into().unwrap());
            let payload_start = offset + 8;
            let payload_end = payload_start + len;
            if payload_end > buf.len() {
                // レコード長が実際のファイルサイズを超える=書き込み途中で
                // 中断された不完全なレコード。ここで読み取りを打ち切る。
                break;
            }
            let payload = &buf[payload_start..payload_end];
            if crc32(payload) != stored_crc {
                // チェックサム不一致=破損レコード。以降は信頼できないため
                // 打ち切る(Consistency、冒頭のモジュールdoc参照)。
                break;
            }
            records.push(payload.to_vec());
            offset = payload_end;
        }
        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rs-smarttcp-wal-test-{tag}-{}.log", std::process::id()))
    }

    #[test]
    fn append_and_read_back_multiple_records_in_order() {
        let path = temp_path("basic");
        let _ = std::fs::remove_file(&path);
        let log = TransactionLog::open(&path).unwrap();

        log.append(b"order: buy 100 shares of AAPL").unwrap();
        log.append(b"order: sell 50 shares of MSFT").unwrap();

        let records = log.read_all_valid_records().unwrap();
        assert_eq!(records, vec![b"order: buy 100 shares of AAPL".to_vec(), b"order: sell 50 shares of MSFT".to_vec()]);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn records_survive_reopening_the_log_atomicity_and_durability() {
        let path = temp_path("reopen");
        let _ = std::fs::remove_file(&path);
        {
            let log = TransactionLog::open(&path).unwrap();
            log.append(b"critical financial record").unwrap();
        }
        // プロセスを再起動したのと同じ状況を模し、新しいハンドルで開き直す。
        let log2 = TransactionLog::open(&path).unwrap();
        let records = log2.read_all_valid_records().unwrap();
        assert_eq!(records, vec![b"critical financial record".to_vec()]);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_truncated_trailing_record_is_ignored_not_returned_as_corrupt_data() {
        let path = temp_path("truncated");
        let _ = std::fs::remove_file(&path);
        {
            let log = TransactionLog::open(&path).unwrap();
            log.append(b"complete record one").unwrap();
        }
        // 2件目の書き込み中にクラッシュした状況を模し、ファイルへ
        // 不完全なレコード(長さヘッダのみ、ペイロード無し)を直接追記する。
        {
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(&999u32.to_le_bytes()).unwrap();
            f.write_all(&0u32.to_le_bytes()).unwrap();
            f.write_all(b"short").unwrap(); // 999バイト分には満たない不完全データ
        }

        let log = TransactionLog::open(&path).unwrap();
        let records = log.read_all_valid_records().unwrap();
        assert_eq!(records, vec![b"complete record one".to_vec()], "the truncated trailing record must be silently ignored, not corrupt the whole read");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_bit_flipped_record_is_detected_via_checksum_and_excluded() {
        let path = temp_path("corrupt");
        let _ = std::fs::remove_file(&path);
        {
            let log = TransactionLog::open(&path).unwrap();
            log.append(b"good record").unwrap();
        }
        // ディスク不良を模し、書き込み済みバイトの1ビットを直接反転する。
        let mut bytes = std::fs::read(&path).unwrap();
        let corrupt_idx = bytes.len() - 1;
        bytes[corrupt_idx] ^= 0xFF;
        std::fs::write(&path, &bytes).unwrap();

        let log = TransactionLog::open(&path).unwrap();
        let records = log.read_all_valid_records().unwrap();
        assert!(records.is_empty(), "a checksum mismatch must exclude the record, not silently return corrupted bytes");

        let _ = std::fs::remove_file(&path);
    }
}
