//! 音質向上目的の帯域固定ポリシー(2026-08-11新設)。
//!
//! ユーザー指示「音質向上目的で、YoutubeやU-NEXTやQobuzなどのオンライン
//! ビデオ・オン・デマンドやオンラインビデオストリーミングや音楽サイトや
//! アプリ利用時は、通信速度を10Mbpsに速度を固定しますか？とチェック
//! ボックスにチェックを付けると機能して、他の通信の利用目的の
//! ホームページやWEBサイトやSFTPやCLAUDEなどのAIやチャットTOOLなどは、
//! 最高速度でアクセス出来るような自動対応の仕様にして」への対応。
//!
//! ## 設計方針(正直な開示)
//!
//! このクレートは実際のパケット送受信を行わない(呼び出し元
//! `open-web-server-wire`等がI/Oを行う)ため、ここで提供するのは
//! 「接続先がストリーミング系かどうかの簡易判定」+「該当する場合の
//! 上限バイトレート算出」という**ポリシー計算のみ**——実際のスロットル
//! (書き込み速度の調整)は、呼び出し側が[`bytes_per_second_limit_for_
//! host`]の結果を見て、自身の送信ループに反映する必要がある。

use std::sync::atomic::{AtomicBool, Ordering};

/// 10Mbps(ISP表記と同じ10進メガビット換算)をバイト/秒に換算した値。
pub const STREAMING_CAP_BYTES_PER_SEC: u64 = 10_000_000 / 8;

/// ユーザー指示で名指しされた、音質向上のため帯域固定の対象となる
/// ストリーミング系サービスのホスト名の一部(部分一致で判定)。
/// **正直な開示**: 網羅的なリストではなく、ユーザーが明示的に挙げた
/// サービスのみを対象とする(推測で対象を広げない)。
const STREAMING_HOST_FRAGMENTS: &[&str] = &["youtube.com", "youtu.be", "unext.jp", "qobuz.com"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrafficPurpose {
    /// 動画・音楽ストリーミング(YouTube/U-NEXT/Qobuz等)。
    Streaming,
    /// それ以外すべて(通常のWebサイト・SFTP・AIチャットツール等)。
    /// 既定でこちらに分類され、常に最高速度(無制限)で扱われる。
    Other,
}

/// 接続先ホスト名からトラフィックの種類を判定する(部分一致、大文字
/// 小文字を無視)。既定は`Other`(最高速度)——ストリーミング系と
/// 明確に一致した場合のみ`Streaming`とする「安全側はOther」の設計
/// (SFTP・Claude等のAI/チャットツールが誤って帯域制限されないため)。
pub fn classify_host(host: &str) -> TrafficPurpose {
    let lower = host.to_lowercase();
    if STREAMING_HOST_FRAGMENTS.iter().any(|f| lower.contains(f)) {
        TrafficPurpose::Streaming
    } else {
        TrafficPurpose::Other
    }
}

/// ユーザーが日英併記のチェックボックスで切り替える「ストリーミング時
/// 10Mbps固定」設定の実行時状態。
pub struct BandwidthPolicy {
    streaming_cap_enabled: AtomicBool,
}

impl BandwidthPolicy {
    pub fn new() -> Self {
        Self { streaming_cap_enabled: AtomicBool::new(false) }
    }

    pub fn set_streaming_cap_enabled(&self, enabled: bool) {
        self.streaming_cap_enabled.store(enabled, Ordering::SeqCst);
    }

    pub fn is_streaming_cap_enabled(&self) -> bool {
        self.streaming_cap_enabled.load(Ordering::SeqCst)
    }

    /// 接続先ホストに対する上限バイトレート。`None`は無制限(最高速度)。
    /// チェックボックスがONかつストリーミング系ホストの場合のみ
    /// `Some(STREAMING_CAP_BYTES_PER_SEC)`を返す——それ以外(通常の
    /// Webサイト・SFTP・AIチャットツール等)は常に`None`(最高速度で
    /// 自動対応)。
    pub fn bytes_per_second_limit_for_host(&self, host: &str) -> Option<u64> {
        if self.is_streaming_cap_enabled() && classify_host(host) == TrafficPurpose::Streaming {
            Some(STREAMING_CAP_BYTES_PER_SEC)
        } else {
            None
        }
    }
}

impl Default for BandwidthPolicy {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_named_streaming_services_correctly() {
        assert_eq!(classify_host("www.youtube.com"), TrafficPurpose::Streaming);
        assert_eq!(classify_host("video.unext.jp"), TrafficPurpose::Streaming);
        assert_eq!(classify_host("play.qobuz.com"), TrafficPurpose::Streaming);
    }

    #[test]
    fn classifies_other_traffic_as_other_by_default() {
        assert_eq!(classify_host("example.com"), TrafficPurpose::Other);
        assert_eq!(classify_host("sftp.example.com"), TrafficPurpose::Other);
        assert_eq!(classify_host("claude.ai"), TrafficPurpose::Other);
        assert_eq!(classify_host("api.anthropic.com"), TrafficPurpose::Other);
    }

    #[test]
    fn cap_only_applies_when_enabled_and_streaming() {
        let policy = BandwidthPolicy::new();
        assert_eq!(policy.bytes_per_second_limit_for_host("www.youtube.com"), None, "off by default");

        policy.set_streaming_cap_enabled(true);
        assert_eq!(policy.bytes_per_second_limit_for_host("www.youtube.com"), Some(STREAMING_CAP_BYTES_PER_SEC));
        assert_eq!(policy.bytes_per_second_limit_for_host("claude.ai"), None, "non-streaming traffic stays uncapped even when the checkbox is on");
    }
}
