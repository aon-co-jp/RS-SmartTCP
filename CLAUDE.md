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

- **2026-08-11(続き11) 「4層4重の通信」+「RS-SmartTCP自体のACID互換
  トランザクション」を新規実装(ユーザー指示「4層4重の通信とACID互換、
  ZFS互換のopen-raid-zとaruaru-db/PostgreSQL/open-web-server/RPoem/
  open-directx/open-cuda/aruaru-llmの連携も可能にして、オンライン証券
  などのネット上のDATAを紛失しない設計思想」→スコープ確認の結果
  「複数WAN/LAN経路で同一データを最大4重送信」+「RS-SmartTCP自体が
  ACIDトランザクションを実装」と確認して着手)**:
  1. **`redundant_transmission.rs`**: 渡された送信クロージャを最大
     [`MAX_REDUNDANT_PATHS`]=4本まで並行実行し、最初に成功した結果を
     採用する`send_redundant`を実装。**正直な開示**: 実際のソケット
     送受信は呼び出し側のクロージャに依存(本クレートはTCP/UDP/QUIC等の
     具体的トランスポートを決め打ちしない既存方針を踏襲)、重複排除
     (複数経路が同時に届いた場合の冪等性)は呼び出し側の責務、
     「データを紛失しない」の保証範囲は「送信の可用性向上」であり
     「永続化の保証」ではないことを明記。
  2. **`transaction_log.rs`**: Write-Ahead Log(WAL)による最小限の
     ACID互換トランザクションログ。`[4バイト長][4バイトCRC32(自前
     実装、外部crate非依存)][ペイロード]`形式で追記、`fsync`相当
     (`File::sync_data()`)でDurabilityを確保。**正直な開示・スコープ
     限定(最重要)**: `open-raid-z`(ZFS互換、RAID冗長化・スナップ
     ショット)や`aruaru-db`/PostgreSQL(複数テーブルにまたがる本格的
     SQLトランザクション)の代替ではなく、「通信層で送信を試みる前に
     ローカルへ確実に記録しておく」ための土台に役割を限定(モジュール
     docに他リポジトリとの役割分担を明記、車輪の再発明を避ける)。
     `redundant_transmission`と組み合わせ「WAL書き込み→複数経路へ
     冗長送信→成功でACK」という設計を想定。
  3. **他リポジトリとの実配線は今回未着手(正直な開示)**: `open-raid-z`/
     `aruaru-db`/PostgreSQL/`open-web-server`/`RPoem`/`open-directx`/
     `open-cuda`/`aruaru-llm`との実際のHTTP/ライブラリ連携コードは
     このセッションでは実装していない——今回はRS-SmartTCP自身が持つ
     べき土台(冗長送信オーケストレーション+WAL)のみを実装し、各
     リポジトリ側との実配線は今後の連携作業として残す(スコープが
     8リポジトリ横断のため、一度に実装せず土台から着手する判断)。
  4. **検証**: `cargo test --lib`**66件全green**(既存58件+
     `redundant_transmission`4件+`transaction_log`4件)。テスト実装中に
     見つけた自己ミス: 最初の`returns_the_first_successful_path_...`
     テストが「最初の成功で即座に返る」設計にもかかわらず「全経路が
     必ず試行完了する」ことを誤ってアサートしており、データレースで
     間欠的に失敗した——設計通りの早期リターン挙動を正しく反映する
     形にテストを修正した。`transaction_log`は実際にファイルへの
     書き込み・プロセス再起動相当の再オープン・不完全レコード
     (書き込み途中で中断)・チェックサム不一致(ビット反転)の4パターン
     全てで正しく動作することを実ファイルI/Oで検証済み。
  - 次にすべきこと: (1) `redundant_transmission`+`transaction_log`を
    `open-web-server-wire`等の実際の呼び出し元から配線する、
    (2) `open-raid-z`側のZFS互換実装との役割分担が実際に噛み合うか
    (WALのcheckpoint/コンパクションをどちらが担うか等)の設計調整、
    (3) dream-osへの同じ設計思想の展開(ユーザー指示、別リポジトリの
    ため別セッションでスコープを切って着手)、(4) open-easy-web
    (全リポジトリ管理)からのこれらモジュールの可視化・管理UIは未着手。

- **2026-08-11(続き10) TLS証明書生成(`tls_inspection.rs`、CA/リーフ発行の
  本実装)+WiFi世代×周波数帯ロードマップメタデータ(`wifi_roadmap.rs`)を
  新設(ユーザー指示「TLS復号・AI侵入検知の本実装して」→スコープ確認の
  上、証明書生成は本実装・実際の復号プロキシ本体は次回以降と正直に
  区切って着手。WiFiは「2.4G/5G/6Gの組み合わせ×WiFi4〜8対応、将来の
  ロードマップを考慮」の日英Web調査+実装指示への対応)**:
  1. **「外部crates.io非依存」原則の一回限りの例外(ユーザー承認済み)**:
     `rcgen`(features: `pem`, `x509-parser`)を追加。TLS証明書生成
     (X.509 CSR構築・署名)はOS標準ツールの組み合わせだけでは安全に
     代替できないため。
  2. **`tls_inspection.rs`**: `ensure_root_ca(dir)`(初回はローカルCAを
     新規生成・保存、以後は再利用——毎回新しいCAを黙って発行しない)・
     `issue_leaf_cert(&ca, hostname)`(SNIホスト名向けリーフ証明書を
     オンデマンド発行)を実装、3件のテストで実際に生成される証明書PEM
     の内容を検証。**正直な開示(最重要のスコープ限定)**: 実際にTCP
     接続を受けてTLSサーバーとして終端し宛先へ再接続する透過プロキシ
     本体(MITMループ処理そのもの)は**未実装**——安全に検証できる範囲
     (証明書生成)のみを「本実装」と区切った。生成したCAを対象端末の
     信頼済みルートストアへ追加する操作も、常にユーザー自身に委ねる
     (本ライブラリはOSの証明書ストアに一切触れない)。
  2. **`wifi_roadmap.rs`**: WiFi4(802.11n)〜WiFi8(802.11bn)の各世代が
     対応する周波数帯(2.4/5/6GHz)をIEEE仕様に基づき定義
     (`WifiGeneration::supported_bands`)、`WifiChannelRegistry`で
     `multi_path`の各WiFiチャンネルへ世代・帯域ラベルを設定・検証
     (IEEE仕様上無効な組み合わせ、例: WiFi5+2.4GHz、は拒否)。
     日英Web調査で裏取り: WiFi 8(IEEE 802.11bn)は**2026-08時点で
     ドラフト段階**(Draft 1.0は2025年7月承認、最終標準化目標2028年9月、
     消費者向け製品の普及は2027〜2028年見込み)であり正式規格ではない
     ことを`WifiGeneration::is_finalized_standard()`で明示——
     「WiFi 8対応」を確定事実として誇張しない。フレッツ光クロス
     (最大10Gbps)向けレンタルルーターのWiFi 7対応開始(2026年5月〜)・
     IOWN/APNとの関係も参考情報としてモジュールdocに記録(実装対象は
     引き続き回線側ではなくWiFi世代/帯域メタデータのみ)。
  3. **複数WAN業者(最大10社)への対応は追加実装不要と確認**:
     `multi_wan.rs`の`MultiWanManager::register_line`は既に
     `MAX_WAN_LINES=10`本まで、各回線に任意の名前(プロバイダ名を含む
     自由文字列)を設定できる設計のため、「WAN業者を最大10社まで」は
     既存の実装がそのまま満たしている(ユーザー確認依頼に対し、
     コード変更ではなく確認で回答)。
  4. **検証**: `cargo test --lib`**58件全green**(既存50件+
     `tls_inspection`3件+`wifi_roadmap`5件)。
  - 次にすべきこと: (1) TLS透過プロキシ本体(TCP accept→TLSサーバー
    終端→宛先へTLSクライアント再接続→双方向バイトコピー)の実装、
    (2) `wifi_roadmap::WifiChannelRegistry`を`multi_path`/GUIへ実配線
    (現状は独立モジュールとして存在するのみ、`status_gui.rs`への
    表示・設定フォームは未実装)、(3) aruaru-llm側の
    `POST /v1/security/classify-traffic`(AI侵入検知、同日実装)を
    RS-SmartTCP側から実際にHTTPで呼ぶ配線(現状はaruaru-llm側の
    エンドポイントのみ実装、RS-SmartTCP側のHTTPクライアント配線は
    次回)。

- **2026-08-11(続き9) `poll_new_drives`をGUIへ実配線+KINGSOFT過大表現の
  追加訂正**:
  1. `examples/status_gui.rs`に「新しく挿入されたドライブを確認」
     ボタン(`POST /check-new-usb`)を追加、`AppState`に
     `usb_seen: Mutex<HashSet<PathBuf>>`と`usb_check_message`を持たせ、
     `usb_protection::poll_new_drives`を実際に呼び出すよう配線した
     (前回HANDOFFの「次にすべきこと」の1点目)。ブラウザ実機で
     ボタン押下→「新しく挿入されたドライブはありません」の応答を
     `curl`で確認済み。**継続する正直な開示**: これはOSイベントフック
     ではなくポーリングであるため、「挿した瞬間」への近さはボタンを
     押す(または呼び出し側アプリが定期的に呼ぶ)頻度に依存する。
  2. ダウンロードファイル保護セクションのKINGSOFT関連の説明文
     (`status_gui.rs`)にも「スキャン画面を開いた」という同種の
     過大表現が残っていたため、`download_protection.rs`側の訂正と
     整合させて「タスクトレイから手動で開いて確認」という表現に修正。
  3. `cargo build --example status_gui`成功、`cargo test --lib`
     **50件全green**、実機起動して`curl`でボタン動作を確認。
  - 次にすべきこと: (1) タスクスケジューラ登録ヘルパーを実際の
    インストーラーから呼ぶかはユーザーと相談の上で判断、(2) TLS
    ディープパケットインスペクション/AI侵入検知は、ユーザーの明確な
    指示により「設定フラグとしてのプラグイン項目のみ」に留め、実際の
    TLS復号・GPU推論の実装は別途スコープを切って着手する方針を再確認
    (今回は着手せず)。

- **2026-08-11(続き8) バックログ3件に着手(タスクスケジューラ登録
  ヘルパー・KINGSOFT `kscan.exe`実起動検証・USB挿入検知のポーリング
  ヘルパー)**:
  1. **`maintenance::windows_scheduled_task_command`/
     `windows_scheduled_task_delete_command`**: Windowsのタスク
     スケジューラへ登録する`schtasks`コマンド文字列を組み立てるだけの
     関数(**本クレート自身はこのコマンドを実行しない**——実行するか
     どうかは呼び出し側アプリの判断。システム設定変更を伴うコマンドを
     このセッション内で無断実行しないという方針を維持)。
  2. **KINGSOFT `kscan.exe`実機検証で重要な事実を発見・訂正**:
     実際に`kscan.exe`を起動して検証したところ、**単独では可視の
     スキャン画面を開かず、常駐プロセス(`kxetray`/`kxescore`)へ
     内部的に指示を送るだけの短命プロセス(起動直後に終了コード0で
     終了)であることが判明**(`EnumWindows`でKINGSOFT関連の可視
     ウィンドウが存在しないことを確認)。これまでの実装コメント・
     メッセージ文言が「スキャン画面を開いた」と誤って断定していたため、
     **正直な開示として、タスクトレイのアイコンから手動でスキャンを
     開始するよう案内する表現に修正した**(`download_protection.rs`の
     `scan_file_kingsoft`)。
  3. **`usb_protection::poll_new_drives`**: 呼び出し側が数秒間隔で
     ポーリングすることで「USBを刺した瞬間」に近い検知を実現する
     ヘルパー(`WM_DEVICECHANGE`のようなOSイベントフックは引き続き
     実装しない、という既存の正直な開示を維持したまま、実用上の
     連携性を向上)。
  4. **検証**: `cargo test --lib`**50件全green**(既存48件+今回追加2件)。
  - 次にすべきこと: (1) `poll_new_drives`を`status_gui.rs`または実際の
    常駐アプリのバックグラウンドタイマーへ実際に組み込む、(2)
    `windows_scheduled_task_command`をインストーラー(open-english等の
    自己インストールスクリプト)から実際に呼び出すかどうかユーザーと
    相談する(システム設定変更を伴うため要確認)。

- **2026-08-11(続き7) ファイル自動実行の要求を明確に拒否+安全な代替
  (「クリーンなら警告解除のみ、実行は常にユーザー操作」)+自動
  メンテナンス機能(セキュリティソフト登録確認+ClamAV定義更新)を追加
  (ユーザー指示「ダウンロードしたファイルを自動実行する?にチェック
  ボックス…AIが自動判定して必要なら自動実行する」を明確に拒否し、
  「スキャンで安全と確認済みのファイルにマークを付け、実行は常に
  ユーザー操作」という代替案で合意)**:
  1. **正直な開示・安全上の判断(最重要)**: 「ダウンロードファイルの
     自動実行」「AIによる自動実行」の両方を明確に拒否した。理由:
     スキャンで「クリーン」という結果は「絶対に安全」を意味しない
     (ゼロデイ・スキャン回避型マルウェアの残存リスク)——自動実行は
     この残存リスクをそのまま実行につなげてしまう。AIが実行主体でも
     この論理は変わらない。
  2. **`download_protection::verify_and_unblock`**: スキャンし、
     `Clean`の場合のみWindows標準の「ブロックの解除」と同じ仕組み
     (Mark of the Web=`Zone.Identifier`代替ストリームの削除)を行う。
     **ファイルを実行する処理は一切含まない**——以後のダブルクリックは
     常にユーザー自身の操作。`ThreatRemoved`の場合は元のパスに
     ファイルが無い(隔離済み)ため何もしない。
  3. **新規`src/maintenance.rs`**: `run_maintenance()`——(a)セキュリティ
     ソフトのWindows Security Center登録確認、(b)ClamAV公式の
     `freshclam`によるウイルス定義更新、を行う。**正直な開示**:
     本クレート自体は起動時・定期実行のスケジューリングを行わない
     (呼び出し側アプリがOSのタスクスケジューラ等から呼ぶ想定)。
     ファイルの自動実行は一切含まない。
  4. **GUI**: スキャンフォームに「クリーンなら安全マーク」チェック
     ボックス(既定ON)+「新しいダウンロードファイルを検知しました。
     自動/手動どちらでスキャンしますか?」の日英併記案内+
     「今すぐメンテナンスを実行」ボタンを追加。
  5. **検証**: `cargo test --lib`**48件全green**(既存47件+今回追加1件、
     `freshclam`未インストールのこの開発機で正直に
     `DefinitionsUpdateUnavailable`を返すことを実機確認)。
  - 次にすべきこと: (1) 呼び出し側アプリでの実際のタスクスケジューラ
    登録、(2) KINGSOFTスキャンGUI実起動の実機確認(前回HANDOFFから
    持ち越し)。

- **2026-08-11(続き6) ダウンロードファイル保護(ウイルススキャン+自動
  実行防止)+USBリムーバブルドライブ保護+セキュリティソフト未導入
  警告を追加(ユーザー指示「コンピューターウイルスの侵入対応と基本的に
  ダウンロードしたファイルの自動実行防止機能…コンピューターウイルスを
  取り除きましたのでファイルは安全です、の英語と日本語のメッセージを
  表示して」→「無料のセキュリティソフトの有名なのと連携してWindowsの
  は辞めましょう」→「オープンソースのClamAV、キングソフト
  セキュリティPro-無料版…選択可能として」→「USBスティックメモリー…
  刺した瞬間にコンピューターウイルスが侵入したり自動実行されたりする
  のを自動でふせいだり…」→「無料版でも良いのでセキュリティソフトを
  インストールしましょう!」)**:
  1. **新規`src/download_protection.rs`**: 独自ウイルス検出エンジンは
     実装せず、無料・オープンソースで有名な**ClamAV**
     (`clamscan`、外部Rustクレート非依存)を実際に呼び出す設計。
     **実機で発見した重要な経緯**: 当初Windows Defender
     (`MpCmdRun.exe`)で試作したが、実機検証でこの開発機のDefender
     オンデマンドスキャンが無効化されていた(`WARN: Product/Feature
     disabled`、`Get-CimInstance -Namespace root/SecurityCenter2`で
     実際に確認したところKINGSOFT Internet Securityが導入されていた
     ことが原因と判明)——特定ベンダー依存を避けるため、ユーザー指示
     によりOS非依存のClamAVへ切り替えた。さらにユーザー指示で
     KINGSOFT(無料版、https://www.kingsoft.jp/is/download)も選択肢に
     追加——ただしKINGSOFTには文書化された自動スキャン用コマンド
     ラインAPIが存在しないため(調査済み)、`ScannerBackend::Kingsoft`
     選択時はスキャン画面(GUI)を開いて手動確認を促すのみに留め、
     「自動で結果取得できる」という誇張はしていない
     (`ScanOutcome::ManualScanRequired`)。
  2. **「駆除」の実装方式**: 完全削除ではなく隔離フォルダへの**移動**
     (`clamscan --move=<dir>`)——可逆的で安全、既存の設計方針と一貫。
     ClamAVの終了コード(`0`=脅威なし・`1`=脅威検出・`2`=エラー、公開
     仕様)に基づき判定し、脅威検出+隔離成功時のみユーザー指示通りの
     「コンピューターウイルスを取り除きましたので、ファイルは安全
     です。」を返す。
  3. **自動実行防止**: Windowsの「Mark of the Web」(実在する
     `<ファイル名>:Zone.Identifier`代替データストリーム)の検出。
     本クレート自身がOSレベルで実行を強制阻止する機能は持たない旨を
     明記。
  4. **新規`src/usb_protection.rs`**: `list_removable_drives()`
     (`Win32_LogicalDisk`の`DriveType=2`、公式WMIクラス)+
     `neutralize_autorun_inf`(削除ではなくリネームによる無害化、
     可逆的)+`protect_drive`(autorun無害化+`download_protection`の
     スキャンをそのまま再利用)。**正直な開示**: WindowsはUSB
     リムーバブルドライブのautorun.inf自動実行を既定で既に無効化
     済み(自動実行は光学メディアのみ)——本機能は念のため残存する
     autorun.infを無害化するもので、USB挿入イベントの常時バック
     グラウンド監視(`WM_DEVICECHANGE`フック等)は実装していない。
  5. **`has_registered_antivirus()`+推奨メッセージ**: Windows
     Security Center(`root/SecurityCenter2`)に何らかのAV製品が
     登録されているかを確認し、1件も無ければ「無料版でも良いので
     インストールしましょう!」を日英併記で表示。**実機で確認**:
     このマシンはKINGSOFT+Defenderの2件が登録されているため、
     この警告が正しく表示されないことを確認済み(誤検知していない
     ことの実証)。
  6. **検証**: `cargo test --lib`**46件全green**(既存39件+今回追加
     7件)。実際に`status_gui`を起動し、ClamAV未インストール時の
     正直な「利用不可」メッセージ表示・USBドライブ0件検出時の表示・
     AV推奨メッセージの非表示(登録済みのため)を`curl`で実HTTP
     検証済み。KINGSOFTのスキャンGUI実起動(`Command::spawn`)は、
     ユーザー環境で意図せずウィンドウを開くことを避けるため今回は
     実行せず、パス検出ロジックの正しさ(`find`コマンドでの事前確認)
     のみ確認済み。
  - 次にすべきこと: (1) KINGSOFTのスキャンGUI実起動の実機確認、
    (2) USB挿入イベントの常時監視(呼び出し側アプリでの`WM_DEVICECHANGE`
    統合)、(3) `open-web-server-wire`側からの実配線は引き続き未着手。

- **2026-08-11(続き5) セキュリティルーター機能に「TLSディープパケット
  インスペクション」「AIベース侵入検知」のプラグイン項目を追加
  (open-cuda/open-directx経由のGPU/NPUアクセラレーター対応、ユーザー
  指示「open-directx + open-cudaでハードウェアがあればハードウェア
  アクセラレーター対応として…必須にしよう」への対応)**:
  - **正直な開示(最重要)**: この2項目は**設定フラグ(有効/無効の
    トグル)としてのみ存在し、実際のTLS復号・再暗号化やGPU/NPU推論に
    よる侵入検知は未実装**。TLSディープパケットインスペクションは
    実質的にユーザー自身のHTTPS通信を復号するMITM(中間者)機能であり、
    実装には証明書チェーンの発行・配布、復号内容の安全な取り扱い、
    `open-cuda`/`open-directx`との実際のGPU/NPU連携配線という重量級
    かつセキュリティ上慎重を要する設計が必要——「有効化ボタンだけ
    用意して中身が空」という誇張を避けるため、この制約を`PluginInfo`
    のラベル文字列自体に明記した(GUI上でも「config flag only, not
    yet implemented / 設定フラグのみ、未実装」と常に表示される)。
  - `cargo test --lib`39件全green(既存の項目追加のみでロジック変更は
    無いため新規テストは追加していない)。実際に`status_gui`を起動し、
    セキュリティルーター機能を有効化した状態でこの2項目が正しく一覧に
    表示されることを実HTTPで確認済み。
  - 次にすべきこと: 実際のTLS復号・GPU/NPU連携は、別途スコープを
    切ってユーザーと実装方針(証明書配布方式・対象トラフィックの範囲・
    プライバシー配慮)を確認した上で着手すべき大きな課題として残す。

- **2026-08-11(続き4) 複数WAN回線対応(最大10本)+LAN/WiFi/Bluetooth
  上限を10へ引き上げ(ユーザー指示「複数WANも最大8本まで対応して」→
  「複数WANも複数LANも最大10本ずつ対応して」→「複数WAN＋複数LAN最大
  10本ずつ＋複数WiFiは最大10チャンネル(回線)＋複数ブルーツースは
  最大10チャンネル(回線)同時接続可能にして」)**:
  1. **新規`src/multi_wan.rs`**: `MultiWanManager`——名前付きの
     [`WanConfig`](単一WAN回線のIPv4/IPv6/v6プラス/自動設定)を最大
     [`MAX_WAN_LINES`]=10本管理する。回線ごとに独立した設定を持てる
     (WAN1はIPv4のまま、WAN2だけv6プラス、のような構成が可能)。
  2. **`MAX_WIRED_PATHS`を4→8→10へ段階的に引き上げ**(ユーザー指示の
     数値変更にそのまま追従)。
  3. **`MAX_WIFI_PATHS`/`MAX_BLUETOOTH_PATHS`を新設**(各10チャンネル)
     ——従来は「複数枚挿さっている環境を想定し上限なし」としていたが、
     ユーザーから明示的に上限数(10)の指定があったため、有線と同じ
     パターンで上限管理に変更した(`from_detected_interfaces`が
     各種別ごとに独立したカウンタで上限を守る)。
  4. **`examples/status_gui.rs`拡張**: WAN回線を名前付きで追加できる
     フォーム+回線ごとの3チェックボックス(自動設定/IPv6/v6プラス)を
     実装。**実機検証**: 実際に2本のWAN回線("WAN1"・"WAN2 (5G
     backup)")を追加し、WAN2だけv6プラスを有効化した状態でWAN1が
     "IPv4"のまま影響を受けないことを`curl`と実ブラウザ操作の両方で
     確認済み(型チェックのみで完了と報告しない方針の徹底)。
  5. **検証**: `cargo test`**39件全green**(既存33件+今回追加6件:
     10チャンネル上限のテスト・複数WAN回線登録テスト等)。
  - 次にすべきこと: (1) `open-web-server-wire`側からの実配線は
    引き続き未着手、(2) WAN回線ごとの`MultiPathManager`的な負荷分散/
    フェイルオーバー(現状は設定フラグの管理のみで、実際に複数WAN回線
    間でトラフィックを振り分けるロジックは未実装)。

- **2026-08-11(続き3) ルーターアプリ機能/セキュリティルーター機能の
  チェックボックス+既知プラグイン一覧+WAN接続方式(IPv4/IPv6/v6プラス)
  設定を追加(ユーザー指示「ルーターアプリ機能＋セキュリティルーター
  機能のそれぞれにチェックを付けられる様にして、チェックを付けると
  追加インストールのプラグインを追加インストール可能にして」→
  「IPV4からIPV6からV6プラス対応にして、WANからの接続を自動設定機能+
  V6プラス機能にチェックボックスを付けたり外したりが可能にして。V6でも
  V6プラス以外の接続も可能にして」)**:
  1. **正直な開示・セキュリティ配慮(最重要)**: 「プラグイン」は任意の
     外部コードをダウンロード・実行する本物のプラグイン機構ではない
     ——ルーター/セキュリティゲートウェイの文脈で未知のコードを実行
     することは重大なセキュリティリスク(サプライチェーン攻撃・任意
     コード実行)であるため、意図的に実装しなかった。代わりに、
     このクレートにあらかじめ組み込まれた既知の機能モジュール(例:
     ポート転送・QoS・DHCPサーバー/広告ブロック・DNSフィルタリング・
     ペアレンタルコントロール)を選んで有効化する設計とした。
  2. **`src/router_features.rs`**: `RouterFeatures`——「ルーターアプリ
     機能」「セキュリティルーター機能」を独立したON/OFFフラグとして
     持ち、それぞれ有効化した場合のみ対応するプラグイン一覧
     (`ROUTER_APP_PLUGINS`/`SECURITY_ROUTER_PLUGINS`)からの
     `install_plugin`/`uninstall_plugin`を許可する(親機能が無効なまま
     プラグインだけ有効化しようとするとエラーを返す)。親機能を無効化
     すると、そのプラグインは自動的にすべてアンインストールされる
     (矛盾した状態を保持しない)。
  3. **`src/wan_config.rs`**: `WanConfig`——「WANからの接続を自動設定」
     「IPv6を使用する」「IPv6 v6プラス(MAP-E)」を独立したフラグとして
     管理。v6プラスを有効化すると自動的にIPv6も有効化されるが、
     IPv6を有効にしてもv6プラスは既定でOFFのまま(ユーザー指示
     「V6でもV6プラス以外の接続も可能にして」への対応、ネイティブ/
     PPPoE方式等のv6プラス以外のIPv6接続も表現できる)。IPv6自体を
     無効化するとv6プラスも連動して無効化される。**正直な開示**:
     実際のDHCPv6-PD交渉・MAP-Eパラメータ取得・トンネル確立は
     OS/ルーター機器側の処理であり、本モジュールは設定意図を表す
     フラグのみを提供する。
  4. **`examples/status_gui.rs`拡張**: チェックボックスの状態変化に
     応じてプラグイン一覧が表示/非表示になる、既存のストリーミング
     チェックボックスと同じ「1チェックボックス=1フォーム、変更時に
     即submit」のUIパターンを踏襲。
  5. **検証**: `cargo test --lib`**33件全green**(既存23件+今回追加
     10件)。実際に`status_gui`を起動し、`curl`でルーターアプリ機能
     有効化→プラグイン一覧表示→QoSプラグイン有効化(チェック状態が
     正しく反映)→v6プラス有効化(現在の方式表示が正しく
     "IPv6 (v6プラス / MAP-E)"に変化)という一連の流れを実際のHTTP
     リクエストと実ブラウザ操作の両方で確認済み。
  - 次にすべきこと: (1) `open-web-server-wire`側からの実配線は
    引き続き未着手、(2) `router_features`/`wan_config`はいずれも
    設定フラグの管理のみで、実際のパケット転送・WAN接続確立ロジックは
    今回のスコープ外(呼び出し側またはOS/ルーター機器が担う)。

- **2026-08-11(続き2) ルーター/NAS/外付けHDD/PC/タブレット/スマホ/TV/
  ゲーム機への経路登録+複数WiFi・複数Bluetooth対応(ユーザー指示
  「ルーターと外付けHDDやNASなどに複数LANケーブル1本から最大4本＋WiFi
  も追加可能にして対応して」→「PC、タブレット、スマホ、TV、ゲーム
  マシンなどと…対応して」→「複数LAN＋複数WiFi＋複数ブルーツゥース
  対応として…対応して」)**:
  1. **`DeviceKind`を新設**(`multi_path.rs`): Router/Nas/
     ExternalStorage/Pc/Tablet/Phone/Tv/GameConsole/Wifi/Bluetooth/
     Otherの11種類。`MultiPathManager::register_device_path(name,
     kind)`でラベル付きの経路登録ができる(経路選択ロジック自体は
     ラベルに関わらずRTTのみで判定、ラベルはGUI表示用)。
  2. **`MultiPathManager::from_detected_interfaces()`新設**:
     `network_interfaces::detect()`の結果から、接続中の有線LAN
     (最大[`MAX_WIRED_PATHS`]=4本)+WiFi(複数枚、上限なし)+
     Bluetooth(複数、上限なし)を自動的に経路登録する——有線のみ
     4本の上限を設け、WiFi/Bluetoothは複数枚挿さっている環境を
     想定し上限を設けない設計。
  3. **`InterfaceKind::Bluetooth`を新設**(`network_interfaces.rs`):
     `Get-NetAdapter`の`PhysicalMediaType`が`"Bluetooth"`のアダプタを
     区別。`wifi_connected_count()`・`bluetooth_connected_count()`
     (複数枚対応のカウント、旧来の`wifi_connected()`は単一WiFiのみを
     前提とした真偽値だったため複数対応の集計メソッドを追加)。
  4. **`examples/status_gui.rs`を拡張**: ルーター/NAS/外付けHDD等の
     IPアドレスを入力すると、TCP接続確立時間(管理者権限不要な簡易
     疎通確認、ICMP pingではないことを明記)を実測し、経路一覧へ
     追加・最良経路を太字ハイライト表示するフォームを実装。
     **実機検証**: 実際のデフォルトゲートウェイ(ルーター、
     `192.168.0.1:80`)へ疎通確認を行い、実測1.5msで正しく最良経路
     判定されることを確認済み(型チェックのみで完了と報告しない
     方針の徹底)。
  5. **検証**: `cargo test`**23件全green**(既存18件+新規5件)。
     実際に`status_gui`を起動し、`curl`・実ブラウザ操作の両方で
     Bluetooth判定・複数WiFi/Bluetoothカウント表示・ルーターへの
     実測疎通確認をすべて確認済み。
  - 次にすべきこと: (1) `open-web-server-wire`側から`MultiPathManager`/
    `BandwidthPolicy`を実際に呼び出す配線は引き続き未着手、(2) PC/
    タブレット/スマホ/TV/ゲーム機の種別は現状「ユーザーが手動でGUIから
    選んでラベル付けする」設計であり、接続先の機器種別を自動判定する
    機能(例: MACアドレスベンダー判定等)は今回のスコープ外。

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
