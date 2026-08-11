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
