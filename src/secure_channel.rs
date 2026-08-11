//! 4層通信セキュリティ(2026-08-11新設、ユーザー指示「通信とDATABASEの
//! 4層4重暗号化セキュリティ通信」への対応)。
//!
//! ## 正直な開示(最重要・依存関係の経緯)
//!
//! このエコシステムには既に`open-web-server-wire::SecureChannel`という
//! 4層防御通信の実装(第1層TLS・第2層相互認証・第3層ChaCha20-Poly1305
//! AEAD・第4層シーケンス番号+タイムスタンプによるリプレイ対策)が実在し、
//! `dream-os-wire`はそれをpath依存でそのまま再利用している(車輪の
//! 再発明を避ける、このエコシステムの一貫した方針)。
//!
//! **しかし`open-web-server-wire`は既に`rs-smarttcp`(このクレート)へ
//! path依存しているため、逆方向の依存(`rs-smarttcp`→
//! `open-web-server-wire`)を追加すると循環依存になりビルドできない**
//! (実際に`cargo add`を試みて`cyclic package dependency`エラーで確認
//! 済み)。そのため本モジュールは、`open-web-server-wire`と**同じ設計
//! (ChaCha20-Poly1305 AEAD+seq/timestampのAAD結合によるリプレイ対策)を、
//! 同じ`chacha20poly1305`クレートを使って独立に実装したもの**——コードの
//! コピーではなく、依存方向の制約により再利用できないため同じ設計思想を
//! 個別に実装した、という経緯を正直に記録する。
//!
//! ## 4層の内訳(このモジュールが担う範囲)
//!
//! - **第1層(TLS)・第2層(相互認証)**: 本モジュールの範囲外。実際の
//!   TCP/QUIC接続確立時にTLS終端・証明書ベースの相互認証を行うのは
//!   呼び出し側アプリ(または[`crate::tls_inspection`]が生成する証明書を
//!   使う経路)の責務。
//! - **第3層(AEAD暗号化)**: `encrypt`/`decrypt`——ChaCha20-Poly1305で
//!   機密性+改ざん検知を提供する。
//! - **第4層(リプレイ対策)**: シーケンス番号+UNIXタイムスタンプをAEADの
//!   Associated Data(AAD)へ暗号学的に結合し、受信側で(1)既知シーケンス
//!   番号の再受信拒否、(2)許容時刻窓外のタイムスタンプ拒否、を行う。
//!   AADに含めているため攻撃者がseq/timestampだけ差し替えて再送しても
//!   AEADタグ検証で失敗する。

use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::RngExt;

/// タイムスタンプの許容ずれ幅(秒)。
pub const FRESHNESS_WINDOW_SECS: u64 = 30;
/// 追跡するシーケンス番号の最大保持件数(メモリ上限)。
const MAX_TRACKED_SEQ: usize = 10_000;

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

struct ReplayGuard {
    seen: BTreeSet<u64>,
}

impl ReplayGuard {
    fn new() -> Self {
        Self { seen: BTreeSet::new() }
    }

    fn check_and_record(&mut self, seq: u64, timestamp_secs: u64) -> Result<(), String> {
        let diff = now_secs().abs_diff(timestamp_secs);
        if diff > FRESHNESS_WINDOW_SECS {
            return Err(format!("timestamp outside freshness window (diff={diff}s)"));
        }
        if !self.seen.insert(seq) {
            return Err(format!("replayed sequence number: {seq}"));
        }
        if self.seen.len() > MAX_TRACKED_SEQ {
            if let Some(&oldest) = self.seen.iter().next() {
                self.seen.remove(&oldest);
            }
        }
        Ok(())
    }
}

/// 第3層(AEAD)+第4層(リプレイ対策)を担う通信チャネル。送信側・受信側
/// 双方が同じ共有鍵で構築する(鍵配送自体は本モジュールの範囲外)。
pub struct SecureChannel {
    cipher: ChaCha20Poly1305,
    next_seq: u64,
    guard: ReplayGuard,
}

impl SecureChannel {
    pub fn new(shared_key: &[u8; 32]) -> Self {
        Self { cipher: ChaCha20Poly1305::new(&Key::try_from(shared_key.as_slice()).expect("32-byte key")), next_seq: 0, guard: ReplayGuard::new() }
    }

    /// 平文を暗号化する。フレーム形式: `[8バイトseq(LE)][8バイト
    /// timestamp(LE)][12バイトnonce][暗号文(タグ込み)]`。
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let seq = self.next_seq;
        self.next_seq += 1;
        let timestamp = now_secs();

        let mut nonce_bytes = [0u8; 12];
        rand::rng().fill(&mut nonce_bytes[..]);
        let nonce = Nonce::try_from(nonce_bytes.as_slice()).expect("12-byte nonce");

        let mut aad = Vec::with_capacity(16);
        aad.extend_from_slice(&seq.to_le_bytes());
        aad.extend_from_slice(&timestamp.to_le_bytes());

        let ciphertext =
            self.cipher.encrypt(&nonce, Payload { msg: plaintext, aad: &aad }).map_err(|e| format!("encryption failed: {e}"))?;

        let mut frame = Vec::with_capacity(28 + ciphertext.len());
        frame.extend_from_slice(&aad);
        frame.extend_from_slice(&nonce_bytes);
        frame.extend_from_slice(&ciphertext);
        Ok(frame)
    }

    /// フレームを検証・復号する。改ざん・リプレイ・鮮度切れのいずれかで
    /// `Err`を返す。
    pub fn decrypt(&mut self, frame: &[u8]) -> Result<Vec<u8>, String> {
        if frame.len() < 28 {
            return Err("frame too short".to_string());
        }
        let seq = u64::from_le_bytes(frame[0..8].try_into().unwrap());
        let timestamp = u64::from_le_bytes(frame[8..16].try_into().unwrap());
        let nonce = Nonce::try_from(&frame[16..28]).expect("12-byte nonce slice");
        let ciphertext = &frame[28..];

        // リプレイ・鮮度チェックを先に行う(復号コストをかける前に弾く)。
        self.guard.check_and_record(seq, timestamp)?;

        let mut aad = Vec::with_capacity(16);
        aad.extend_from_slice(&frame[0..16]);

        self.cipher.decrypt(&nonce, Payload { msg: ciphertext, aad: &aad }).map_err(|e| format!("decryption/authentication failed: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_encrypt_decrypt_recovers_original_plaintext() {
        let key = [1u8; 32];
        let mut sender = SecureChannel::new(&key);
        let mut receiver = SecureChannel::new(&key);

        let frame = sender.encrypt(b"transfer $100 to account X").unwrap();
        let plaintext = receiver.decrypt(&frame).unwrap();
        assert_eq!(plaintext, b"transfer $100 to account X");
    }

    #[test]
    fn replayed_frame_is_rejected_on_second_delivery() {
        let key = [2u8; 32];
        let mut sender = SecureChannel::new(&key);
        let mut receiver = SecureChannel::new(&key);

        let frame = sender.encrypt(b"order: buy 10 shares").unwrap();
        assert!(receiver.decrypt(&frame).is_ok());
        assert!(receiver.decrypt(&frame).is_err(), "replaying the exact same frame must be rejected");
    }

    #[test]
    fn tampered_frame_fails_authentication_instead_of_decrypting_garbage() {
        let key = [3u8; 32];
        let mut sender = SecureChannel::new(&key);
        let mut receiver = SecureChannel::new(&key);

        let mut frame = sender.encrypt(b"critical data").unwrap();
        let last = frame.len() - 1;
        frame[last] ^= 0xFF;
        assert!(receiver.decrypt(&frame).is_err(), "a tampered ciphertext must fail AEAD authentication");
    }

    #[test]
    fn wrong_key_cannot_decrypt() {
        let mut sender = SecureChannel::new(&[4u8; 32]);
        let mut wrong_receiver = SecureChannel::new(&[5u8; 32]);

        let frame = sender.encrypt(b"secret").unwrap();
        assert!(wrong_receiver.decrypt(&frame).is_err());
    }
}
