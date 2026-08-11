# 開発方針・開発環境ルール(RS-SmartTCP)

作業ドライブは`F:\runo`。この節は[`open-raid-z`](https://github.com/aon-co-jp/open-raid-z)の
`CLAUDE.md`を正本とし、各プロジェクトへコピーして同期する方針に準じる。
GitHubリポジトリ: [aon-co-jp/RS-SmartTCP](https://github.com/aon-co-jp/RS-SmartTCP)。

## このプロジェクトの役割

IOWN/APN(NTTのオールフォトニクス・ネットワーク)のような超低遅延・
ジッター無し回線と、Smart-TCP(AI生成通信プロトコル)の良いとこ取り
ハイブリッド適応制御。`open-web-server-wire`から利用される、
`Rust-JSON`と同じ「独立リポジトリとして切り出し、必要な場所から
path依存する」パターンの一員。

## 正直な開示(最重要)

> ⚠️ **本クレートは、arXiv 2512.00491("Agentic AI-based Autonomous
> and Adaptive TCP Protocol"、"Smart-TCP")のプロトコルそのものの
> 実装ではない。** 訓練済み機械学習モデルは使わず、「fast/slowモデル」
> という設計思想を、TCP(RFC 6298)/QUIC(RFC 9002)が実際に使う
> SRTT/RTTVAR(Jacobson/Karels EWMA)に基づく決定論的な2値判定として
> 実装したもの。名前を`Smart-TCP`ではなく`RS-SmartTCP`にしたのは、
> 実在する同名論文との混同を避けるため(ユーザー確認済み、2026-07-23)。
>
> IOWN/APN自体もNTTが構築する物理telecom基盤(光電融合スイッチ・
> 光ファイバー回線)であり、本クレートが「実装」できる対象ではない
> ——実際に行っているのは「そのような回線が来た時にソフトウェア層が
> 足を引っ張らない」設計のみ。

## 技術スタック

外部依存クレート無し(標準ライブラリのみ)。`unsafe`不使用。

## HANDOFF

- **2026-08-11(続き) アダプタ表示名の文字化けを根本解消
  (`ipconfig`解析 → `Get-NetAdapter`へ変更、ユーザー報告「イーサネット
  アダプターの文字の下の行などが全ての行で文字化けして読めません」への
  対応)**:
  1. **前回の対応(`chcp 65001`)では不十分だったことが判明**: 見出し
     行の判定・接続本数のカウントは直っていたが、アダプタの表示名
     自体(「イーサネット」等)はなお文字化けしたまま——`ipconfig`が
     ローカライズ済み名称を内部的に別経路でレンダリングしていたと
     見られる。
  2. **根本対応**: `ipconfig`のテキスト解析自体をやめ、PowerShellの
     `Get-NetAdapter`(`[Console]::OutputEncoding`をUTF-8に明示設定)
     から`Name||Status||PhysicalMediaType`形式で取得する方式へ変更。
     `PhysicalMediaType`(`"802.3"`=有線、`"Native 802.11"`=WiFi)は
     ロケールに依存しない英数字の技術定数のため、種別判定自体も
     文字化け問題から原理的に解放される。
  3. **副次的な精度向上**: 旧方式は"Bluetooth ネットワーク接続"アダプタ
     を名前に"ethernet"の文字列が含まれないため`Other`扱いにできて
     いたが、たまたま`Ethernet`の文字列マッチに引っかかる名前だと
     誤分類する可能性があった。新方式は`PhysicalMediaType`という
     専用フィールドで判定するため、この種の誤分類が構造的に起きない。
  4. **実機検証**: `cargo test`**18件全green**(既存16件+新規2件:
     `parses_wifi_adapter_by_physical_media_type_not_connected`・
     `preserves_japanese_adapter_names_without_mojibake`)。実際に
     `status_gui`を再起動し、`curl`・実ブラウザ操作の両方で
     「イーサネット」「イーサネット 2」「イーサネット 3」
     「Bluetooth ネットワーク接続」がすべて文字化け無く正しく表示
     されることを確認済み。
  - 次にすべきこと: 特になし(今回の報告は解消済み)。

- **2026-08-11 有線LAN最大4本+WiFi同時接続の最良経路選択・自動
  フェイルオーバー+動作確認用GUI+ストリーミング時10Mbps固定機能を追加
  (ユーザー指示「LANコネクターが仮にUSBであろうと、PCIE経由であろうと
  マザーボード経由でもLANケーブルは最大4本＋Wifi同時接続で通信の高速化
  と安定化機能を搭載して。機能確認の為のGUI化と、今何がつながっているか
  の確認機能も付けて。音質向上目的で、YoutubeやU-NEXTやQobuzなどの…
  利用時は、通信速度を10Mbpsに速度を固定しますか？とチェックボックスに
  チェックを付けると機能して、他の通信の利用目的の…SFTPやCLAUDEなどの
  AIやチャットTOOLなどは、最高速度でアクセス出来るような自動対応の
  仕様にして」への対応)**:
  1. **正直な開示(最重要)**: 本物の帯域合算リンクアグリゲーション
     (複数回線の速度を足し合わせて1本の高速回線のように使う機能)は
     実装していない——OS/NICドライバのチーミング機能かMPTCP(Linux
     カーネル)を必要とし、コンシューマ版Windows上のユーザー空間Rust
     ライブラリからは実現できないため。実際に実装したのは
     「最良経路選択(高速化)」+「自動フェイルオーバー(安定化)」の
     2点であり、この区別をコード・GUI双方に明記した。
  2. **`src/network_interfaces.rs`**: `ipconfig`の出力解析
     (`std::process::Command`のみ、外部クレート非依存の既存方針を
     維持)でWindows上の有線LAN・WiFiの接続本数を検出。バス種別
     (USB/PCIe/オンボード)はWindows標準APIでは区別できないため
     区別しない旨を明記(正直な開示)。**実機検証で発見・修正した
     実バグ**: 日本語版Windowsの`ipconfig`出力はShift-JIS(cp932)
     エンコーディングで出力され、素朴に`String::from_utf8_lossy`で
     解釈すると文字化けし種別判定(Ethernet/Wifi)にも失敗していた
     ——`cmd /C chcp 65001 >nul && ipconfig`でコードページを一時的に
     UTF-8へ切り替えることで、見出し行の判定・接続本数のカウントは
     正しく動作するよう修正(アダプタの表示名自体は既知の残存制限として
     コード内に明記、機能面には影響しない)。
  3. **`src/multi_path.rs`**: `MultiPathManager`——経路(有線LAN最大4本+
     WiFi)ごとに個別の`NetworkQualityMonitor`を保持し、最もRTTが低い
     経路を`best_path()`で選択、劣化時は自動的に次に良い経路へ
     フェイルオーバーする。
  4. **`src/bandwidth_policy.rs`**: `BandwidthPolicy`——ユーザーが
     チェックボックスで切り替える「ストリーミング時10Mbps固定」設定。
     `classify_host()`はホスト名がYouTube/U-NEXT/Qobuz(ユーザーが
     名指ししたサービスのみ、推測で対象を広げない)に一致する場合のみ
     `Streaming`、それ以外(通常のWebサイト・SFTP・Claude等のAI/
     チャットツール含む)は既定で`Other`(常に無制限=最高速度)と
     分類する「安全側はOther」設計。
  5. **`examples/status_gui.rs`**: 動作確認用の最小限GUI
     (`std::net`のみ、外部Webフレームワーク非依存)。接続状況の表示+
     ストリーミング固定チェックボックス(日英併記)を実装。
  6. **検証**: `cargo test`**16件全green**(既存6件+今回追加10件)。
     実際に`status_gui`を起動し、`curl`および実ブラウザ操作(チェック
     ボックスのクリック)の両方で動作確認済み(型チェックのみで完了と
     報告しない方針の徹底)。
  - 次にすべきこと: (1) アダプタ表示名の文字化けを完全に解消するには
    `ipconfig`のテキスト解析から`PowerShell Get-NetAdapter`の構造化
    出力へ切り替える必要がある(機能面への影響は無いため優先度は低い)、
    (2) `open-web-server-wire`側から`BandwidthPolicy`/`MultiPathManager`
    を実際に呼び出す配線(現状は独立クレートとして存在するのみ、
    呼び出し元は未接続)、(3) `bytes_per_second_limit_for_host`の
    結果を実際の送信ループへ反映するトークンバケット式スロットル実装
    (現状はポリシー計算のみ、実際のI/O速度調整は呼び出し側の責務)。

- **2026-07-23 新規作成**: ユーザー指示「光のプロトコルというAIが
  生み出した通信プロトコルの良いとこ取りハイブリッド対応」を受けて
  着手。日英Web検索で以下を裏取り:
  - IOWN/APN: 日本-台湾間3,000kmで約17ms・ジッター無しを実証済み
    ([digitimes: NTT IOWN 2026](https://www.digitimes.com/news/a20251007PD227/ntt-iown-infrastructure-launch-2026.html))。
  - Smart-TCP: arXiv 2512.00491、"Agentic AI-based Autonomous and
    Adaptive TCP Protocol"、fast/slowの2モデルによる判断構造。
  - RTT/ジッター推定の実装方式: 当初は固定ウィンドウ+標準偏差方式で
    実装したが、ユーザー指示による再検証の結果、TCP(RFC 6298)/
    QUIC(RFC 9002)が実際に使うSRTT/RTTVAR(Jacobson/Karels EWMA)へ
    書き換えた——O(1)更新で済み、かつこのエコシステムが既に使う
    QUICの輻輳制御と同じ枯れたアルゴリズムであるため。
  - 当初`open-web-server-wire`内の`adaptive_channel`モジュールとして
    実装したが、`Rust-JSON`と同じ「独立リポジトリへ切り出し、path依存
    する」パターンに合わせ、このリポジトリへ切り出した。
  - **検証**: `cargo test` 6件全green(photonic-class/standard-class
    判定・RFC 6298初回サンプル特別扱い・Fast/Slowモード切替・
    ジッター増加時のFastからSlowへの降格を実証)。
  - VPS(`ssh conoha`、`/root/RS-SmartTCP`)へもclone・ビルド確認済み。
  - 次にすべきこと: (1) `open-web-server-ledger::Ledger`の
    `retry_backoff`を`AdaptivePolicy`経由に実際に配線する(現状は
    独立クレートとして存在するのみ、呼び出し元は未接続)、
    (2) `open-web-server-wire::udp_channel`等の他の通信層からも
    `NetworkQualityMonitor`を使う配線の検討。

## 関連プロジェクト

- [open-raid-z](https://github.com/aon-co-jp/open-raid-z) — 開発ルールの正本。
- [open-web-server](https://github.com/aon-co-jp/open-web-server) — 本クレートの利用元
  (`open-web-server-wire::accel`/通信層四重化)。
- [RPoem](https://github.com/aon-co-jp/RPoem) — Apache+Tomcatに例えると
  Tomcat役、`open-web-server`と対になるアプリケーションサーバー層。

## エコシステム全体マップ

同時並行開発の対象プロジェクト一覧・各リポジトリの現況は
[`open-raid-z`のCLAUDE.md](https://github.com/aon-co-jp/open-raid-z/blob/main/CLAUDE.md)
「関連プロジェクト」節を参照。
