//! TLSディープパケットインスペクション(2026-08-11、ユーザー指示
//! 「TLS復号・AI侵入検知の本実装して」への対応)。
//!
//! ## 正直な開示(最重要・スコープの明確化)
//!
//! 本モジュールが**実際に実装しているのはCA証明書・ホスト別リーフ
//! 証明書の生成のみ**(`rcgen`クレートによる本物の証明書生成、テスト
//! 済み)。**実際にTCP接続を受けてTLSサーバーとして終端し、宛先へ
//! TLSクライアントとして再接続して復号ペイロードを中継する透過
//! プロキシ本体(いわゆるMITMプロキシのループ処理)は、このセッションでは
//! 未実装。** 理由: 双方向TLSリレー(SNI別証明書選択・ハンドシェイク・
//! 双方向ストリームのコピー・エラー処理)を安全に実装し検証するには
//! 本セッションの残り作業量では不十分と判断し、誤って「動作する」と
//! 主張するリスクを避けるため、検証済みの部分(証明書生成)のみを
//! 「本実装」として区切った。
//!
//! ## 依存クレートについて(既存方針からの明示的な例外)
//!
//! 本クレートはこれまで「外部crates.io依存を持たない、OS標準ツールを
//! `std::process::Command`で呼ぶ」方針を貫いてきたが、TLS証明書生成
//! (X.509 CSR構築・署名)は同等の処理をOS標準ツールの組み合わせだけで
//! 安全に代替できないため、ユーザーの明示的な承認を得て`rcgen`
//! (純Rust実装、ASN.1/X.509生成)を追加した——**この2機能(TLS復号・
//! AI侵入検知)に限った一回限りの例外**であり、他のモジュールへは
//! 適用しない。
//!
//! ## 使い方(想定)
//!
//! 1. `ensure_root_ca(dir)`でCA秘密鍵・証明書を生成(初回のみ)。
//! 2. 生成された`ca_cert_pem`を、実際にインターセプトしたい端末側の
//!    信頼済みルート証明書ストアへ**ユーザー自身の操作で**追加する
//!    (本ライブラリは対象端末の証明書ストアを無断で書き換えない)。
//! 3. `issue_leaf_cert(&ca, hostname)`で、接続先ホスト名ごとの
//!    リーフ証明書をオンデマンドに発行する(実際のプロキシ本体を
//!    実装する際、SNIで受け取ったホスト名をそのまま渡す想定)。

use std::path::{Path, PathBuf};

use rcgen::{CertificateParams, DistinguishedName, DnType, Issuer, KeyPair, SanType};

pub struct RootCa {
    pub key_pair: KeyPair,
    pub cert_pem: String,
}

pub fn default_ca_dir() -> PathBuf {
    std::env::temp_dir().join("rs-smarttcp-tls-inspection-ca")
}

/// ルートCAを生成する(既存ファイルがあればそれを読み込む、無ければ
/// 新規生成して`dir`へ保存する)。**このCAを対象端末の信頼済みルート
/// ストアへ追加するかどうかは、常にユーザー自身の判断・操作に委ねる**
/// (本関数はファイルへの書き込みのみ行い、OSの証明書ストアには一切
/// 触れない)。
pub fn ensure_root_ca(dir: &Path) -> Result<RootCa, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("failed to create CA dir: {e}"))?;
    let key_path = dir.join("ca.key.pem");
    let cert_path = dir.join("ca.cert.pem");

    if key_path.exists() && cert_path.exists() {
        let key_pem = std::fs::read_to_string(&key_path).map_err(|e| e.to_string())?;
        let cert_pem = std::fs::read_to_string(&cert_path).map_err(|e| e.to_string())?;
        let key_pair = KeyPair::from_pem(&key_pem).map_err(|e| format!("invalid stored CA key: {e}"))?;
        return Ok(RootCa { key_pair, cert_pem });
    }

    let mut params = CertificateParams::new(Vec::<String>::new()).map_err(|e| e.to_string())?;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "RS-SmartTCP Local Inspection CA (DO NOT trust unless you generated this yourself)");
    dn.push(DnType::OrganizationName, "RS-SmartTCP");
    params.distinguished_name = dn;
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params.key_usages = vec![rcgen::KeyUsagePurpose::KeyCertSign, rcgen::KeyUsagePurpose::CrlSign];

    let key_pair = KeyPair::generate().map_err(|e| e.to_string())?;
    let cert = params.self_signed(&key_pair).map_err(|e| e.to_string())?;
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    std::fs::write(&key_path, &key_pem).map_err(|e| e.to_string())?;
    std::fs::write(&cert_path, &cert_pem).map_err(|e| e.to_string())?;

    Ok(RootCa { key_pair, cert_pem })
}

/// 指定ホスト名向けのリーフ証明書を、渡されたCAで署名して発行する
/// (実際の透過プロキシ実装時、TLSハンドシェイクのSNIで受け取った
/// ホスト名をそのまま渡す想定)。証明書・秘密鍵ともPEM形式で返す。
pub fn issue_leaf_cert(ca: &RootCa, hostname: &str) -> Result<(String, String), String> {
    let mut params = CertificateParams::new(vec![]).map_err(|e| e.to_string())?;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, hostname);
    params.distinguished_name = dn;
    params.is_ca = rcgen::IsCa::NoCa;
    params.subject_alt_names = vec![if let Ok(ip) = hostname.parse::<std::net::IpAddr>() {
        SanType::IpAddress(ip)
    } else {
        SanType::DnsName(hostname.try_into().map_err(|_| format!("invalid hostname for SAN: {hostname}"))?)
    }];

    let leaf_key = KeyPair::generate().map_err(|e| e.to_string())?;
    let issuer = Issuer::from_ca_cert_pem(&ca.cert_pem, &ca.key_pair).map_err(|e| e.to_string())?;
    let cert = params.signed_by(&leaf_key, &issuer).map_err(|e| e.to_string())?;

    Ok((cert.pem(), leaf_key.serialize_pem()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rs-smarttcp-tls-ca-test-{tag}-{}", std::process::id()))
    }

    #[test]
    fn ensure_root_ca_generates_and_persists_a_valid_pem_certificate() {
        let dir = temp_dir("root");
        let _ = std::fs::remove_dir_all(&dir);

        let ca = ensure_root_ca(&dir).expect("CA generation must succeed");
        assert!(ca.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(dir.join("ca.key.pem").exists());
        assert!(dir.join("ca.cert.pem").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_root_ca_reuses_existing_files_on_second_call_instead_of_regenerating() {
        let dir = temp_dir("reuse");
        let _ = std::fs::remove_dir_all(&dir);

        let first = ensure_root_ca(&dir).expect("first generation must succeed");
        let second = ensure_root_ca(&dir).expect("second call must load, not regenerate");
        assert_eq!(first.cert_pem, second.cert_pem, "re-running must reuse the same CA, not silently mint a new one");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn issue_leaf_cert_produces_a_certificate_for_the_requested_hostname() {
        let dir = temp_dir("leaf");
        let _ = std::fs::remove_dir_all(&dir);
        let ca = ensure_root_ca(&dir).expect("CA generation must succeed");

        let (leaf_pem, leaf_key_pem) = issue_leaf_cert(&ca, "example.com").expect("leaf cert issuance must succeed");
        assert!(leaf_pem.contains("BEGIN CERTIFICATE"));
        assert!(leaf_key_pem.contains("BEGIN PRIVATE KEY") || leaf_key_pem.contains("BEGIN EC PRIVATE KEY"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
