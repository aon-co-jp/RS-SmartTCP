//! ダウンロードファイルの自動実行防止+ウイルススキャン(2026-08-11新設)。
//!
//! ユーザー指示「コンピューターウイルスの侵入対応と基本的にダウンロード
//! したファイルの自動実行防止機能。ダウンロードするファイルを通過する
//! 時に圧縮でも自動スキャンしてコンピューターウイルスが含まれていれば、
//! コンピューターウイルスを取り除きましたのでファイルは安全です。の
//! 英語と日本語のメッセージを表示して」→「無料のセキュリティソフトの
//! 有名なのと連携してWindowsのは辞めましょう」への対応。
//!
//! ## 正直な開示(最重要)
//!
//! - **独自のウイルス検出エンジンは実装していない。** シグネチャ
//!   データベースの構築・維持は現実的な範囲を超えるため、無料・
//!   オープンソースで世界的に有名な**ClamAV**(`clamscan`、
//!   `std::process::Command`経由、外部Rustクレート非依存)をそのまま
//!   呼び出す設計とした——「スキャンしたふりをする」誇張は行わない。
//! - **経緯**: 当初Windows Defender(`MpCmdRun.exe`)を使う実装を
//!   試みたが、実機検証でこの開発機ではDefenderのオンデマンドスキャン
//!   機能自体が無効化されていた(`WARN: Product/Feature disabled`、
//!   別のセキュリティ製品〈KINGSOFT Internet Security〉が導入されて
//!   いたため、実際に`Get-CimInstance -Namespace root/SecurityCenter2
//!   -ClassName AntivirusProduct`で確認済み)。特定ベンダーのOS標準
//!   AV製品に依存すると、ユーザーの環境によって動かないという同じ
//!   問題が起きうるため、ユーザー指示によりOS依存のWindows Defender
//!   ではなく、Windows/Linux両対応でどの環境でも同じように動く
//!   ClamAVへ切り替えた。
//! - **ClamAV自体は同梱しない**: `clamscan`実行ファイルが見つからない
//!   場合は`ScanOutcome::ScannerUnavailable`を正直に返す(黙って
//!   「安全」と判定しない)——利用にはユーザー自身が
//!   [clamav.net](https://www.clamav.net/)から無料でダウンロード・
//!   インストールする必要がある。
//! - **圧縮ファイル(ZIP/7z等)の中身の展開は自前で実装していない。**
//!   ClamAVのエンジン自体が主要なアーカイブ形式の内部までスキャンする
//!   機能を持つため、本モジュールはアーカイブファイルをそのまま
//!   `clamscan`へ渡すだけであり、独自の解凍ロジックは持たない。
//! - **「駆除」の実装方式**: 完全削除(`--remove`)ではなく、
//!   隔離フォルダへの**移動**(`--move=<quarantine_dir>`)を既定とする
//!   ——元の場所からは無くなるため「ファイルは安全です」を正直に
//!   主張できる一方、完全削除より可逆的で安全(ユーザーが誤検知だと
//!   判断すれば隔離フォルダから戻せる)。
//! - **自動実行防止**: Windowsが実際にダウンロード済みファイルへ付与
//!   する「Mark of the Web」(`<ファイル名>:Zone.Identifier`という
//!   代替データストリーム)の有無を検出する、実在するWindowsの
//!   セキュリティ機構。**本クレート自身がOSの実行そのものを強制的に
//!   ブロックする機能は持たない**——`should_block_execution_until_
//!   scanned`はあくまで呼び出し側が実行前に判断するための材料。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 選択可能なスキャナー(2026-08-11追加、ユーザー指示「オープンソースの
/// ClamAV、キングソフト セキュリティPro-無料版…などは選択可能として」
/// への対応)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScannerBackend {
    ClamAv,
    /// キングソフト インターネットセキュリティ(無料版、
    /// https://www.kingsoft.jp/is/download)。
    Kingsoft,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanOutcome {
    /// 脅威は見つからなかった(元から安全)。
    Clean,
    /// 脅威が見つかり、隔離フォルダへ移動した(元の場所からは除去済み)。
    ThreatRemoved,
    /// 脅威が見つかったが、隔離に失敗した(手動対応が必要)。
    ThreatFoundNotRemoved,
    /// スキャナーが見つからない・実行できない(実機検証で確認した
    /// 実際に起こりうる状態、黙って「安全」とは判定しない)。
    ScannerUnavailable,
    /// スキャナー自体は見つかったが、自動スキャン用のコマンドライン
    /// インターフェースを持たないため、GUIを開いて手動スキャンを
    /// 促すに留めた(KINGSOFT向け、正直な開示——自動化した結果取得は
    /// できていない)。
    ManualScanRequired,
}

#[derive(Debug, Clone)]
pub struct ScanResult {
    pub outcome: ScanOutcome,
    pub message_en: String,
    pub message_ja: String,
}

/// `clamscan`実行ファイルの候補パス(Windows版ClamAVの一般的な
/// インストール先)+PATH上の`clamscan`。
fn find_clamscan() -> Option<String> {
    if Command::new("clamscan").arg("--version").output().is_ok() {
        return Some("clamscan".to_string());
    }
    const CANDIDATES: &[&str] = &[r"C:\Program Files\ClamAV\clamscan.exe", r"C:\ClamAV\clamscan.exe"];
    CANDIDATES.iter().find(|p| Path::new(p).exists()).map(|s| s.to_string())
}

/// キングソフト インターネットセキュリティのスキャン画面
/// (`kscan.exe`)の候補パス。**正直な開示**: 2026-08-11時点の調査で、
/// KINGSOFT製品には公開・文書化された自動スキャン用コマンドライン
/// インターフェースが存在しないことを確認済み——見つかった場合は
/// スキャン画面を開くのみで、結果の自動取得は行わない。
fn find_kingsoft_scan_ui() -> Option<String> {
    const CANDIDATES: &[&str] = &[
        r"C:\Program Files (x86)\Kingsoft\kingsoft security pro\kscan.exe",
        r"C:\Program Files\Kingsoft\kingsoft security pro\kscan.exe",
    ];
    CANDIDATES.iter().find(|p| Path::new(p).exists()).map(|s| s.to_string())
}

/// `clamscan`の終了コード+標準出力からスキャン結果を解釈する
/// (テスト容易性のため実行部分と分離)。ClamAVの終了コードは
/// 安定した公開仕様: `0`=脅威なし、`1`=脅威検出、`2`=エラー。
pub fn interpret_scan_result(exit_code: Option<i32>, stdout: &str, moved_to_quarantine: bool) -> ScanResult {
    match exit_code {
        Some(0) => ScanResult {
            outcome: ScanOutcome::Clean,
            message_en: "No computer virus was found. This file is safe.".to_string(),
            message_ja: "コンピューターウイルスは見つかりませんでした。このファイルは安全です。".to_string(),
        },
        Some(1) => {
            if moved_to_quarantine {
                ScanResult {
                    outcome: ScanOutcome::ThreatRemoved,
                    message_en: "A computer virus was found and removed. This file is now safe.".to_string(),
                    message_ja: "コンピューターウイルスを取り除きましたので、ファイルは安全です。".to_string(),
                }
            } else {
                ScanResult {
                    outcome: ScanOutcome::ThreatFoundNotRemoved,
                    message_en: "A computer virus was found but could not be quarantined. Do not open this file. / コンピューターウイルスが見つかりましたが、隔離できませんでした。このファイルは開かないでください。".to_string(),
                    message_ja: String::new(),
                }
            }
        }
        _ => ScanResult {
            outcome: ScanOutcome::ScannerUnavailable,
            message_en: format!(
                "Virus scan could not be completed (scanner error). Raw output: {}",
                stdout.lines().last().unwrap_or("").trim()
            ),
            message_ja: "ウイルススキャンを完了できませんでした(スキャナーエラー)。".to_string(),
        },
    }
}

/// ファイルをスキャンする(バックエンド選択式、2026-08-11追加)。
pub fn scan_file(backend: ScannerBackend, path: &Path, quarantine_dir: &Path) -> ScanResult {
    match backend {
        ScannerBackend::ClamAv => scan_file_clamav(path, quarantine_dir),
        ScannerBackend::Kingsoft => scan_file_kingsoft(),
    }
}

/// KINGSOFT インターネットセキュリティ(無料版)向け。**正直な開示**:
/// 自動スキャン用のコマンドラインAPIが存在しないため、実際に結果を
/// 自動判定することはできない——インストール済みなら`kscan.exe`の起動を
/// 試みるが、**2026-08-11に実機で検証した結果、`kscan.exe`は常駐する
/// バックグラウンドプロセス(`kxetray`/`kxescore`)へ内部的に指示を
/// 送るだけの短命プロセスであり、単独では可視のスキャン画面を開かない
/// ことを確認した**(起動直後に終了コード0で終了、`EnumWindows`で
/// KINGSOFT関連の可視ウィンドウは検出できず)。そのため「スキャン画面が
/// 開いた」とは主張せず、タスクトレイ(`kxetray`)アイコンから手動で
/// スキャンを開始するよう正直に案内する。未インストールの場合は無料
/// ダウンロードページ(ユーザー提供のURL、
/// https://www.kingsoft.jp/is/download)を案内する。
fn scan_file_kingsoft() -> ScanResult {
    match find_kingsoft_scan_ui() {
        Some(exe) => {
            let _ = Command::new(exe).spawn();
            ScanResult {
                outcome: ScanOutcome::ManualScanRequired,
                message_en: "KINGSOFT Internet Security does not provide a documented automatic command-line scan interface. A scan trigger was sent, but it may not open a visible window on its own — please open KINGSOFT from its system tray icon and run the scan manually, then confirm the result yourself.".to_string(),
                message_ja: "キングソフト インターネットセキュリティには、文書化された自動コマンドラインスキャン機能がありません。スキャンの起動を試みましたが、単独では画面が表示されない場合があります。タスクトレイのキングソフトアイコンから手動でスキャンを実行し、結果はご自身でご確認ください。".to_string(),
            }
        }
        None => ScanResult {
            outcome: ScanOutcome::ScannerUnavailable,
            message_en: "KINGSOFT Internet Security was not found. Download the free edition from https://www.kingsoft.jp/is/download . / キングソフト インターネットセキュリティが見つかりませんでした。https://www.kingsoft.jp/is/download から無料版をダウンロードできます。".to_string(),
            message_ja: String::new(),
        },
    }
}

/// ファイルを実際にClamAVでスキャンし、脅威が見つかれば隔離フォルダへ
/// 移動する。`clamscan`が見つからない場合は`ScanOutcome::
/// ScannerUnavailable`を返す(パニックしない、黙って安全と判定しない)。
/// 圧縮ファイルの中身の展開はClamAV自身のエンジンに委ねる(本関数は
/// ファイルパスを渡すだけ)。
fn scan_file_clamav(path: &Path, quarantine_dir: &Path) -> ScanResult {
    let Some(clamscan) = find_clamscan() else {
        return ScanResult {
            outcome: ScanOutcome::ScannerUnavailable,
            message_en: "Virus scan unavailable: ClamAV (clamscan) was not found. Install it for free from https://www.clamav.net/. / ウイルススキャンは利用できません: ClamAV(clamscan)が見つかりませんでした。https://www.clamav.net/ から無料でインストールできます。".to_string(),
            message_ja: String::new(),
        };
    };

    let _ = std::fs::create_dir_all(quarantine_dir);
    let move_arg = format!("--move={}", quarantine_dir.display());

    match Command::new(&clamscan).args(["--infected", &move_arg]).arg(path).output() {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let exit_code = out.status.code();
            let moved_to_quarantine = quarantine_dir.join(path.file_name().unwrap_or_default()).exists();
            interpret_scan_result(exit_code, &stdout, moved_to_quarantine)
        }
        Err(e) => ScanResult {
            outcome: ScanOutcome::ScannerUnavailable,
            message_en: format!("Virus scan unavailable: failed to run ClamAV ({e}). / ウイルススキャンは利用できません: ClamAVの実行に失敗しました({e})。"),
            message_ja: String::new(),
        },
    }
}

/// 既定の隔離フォルダ(このライブラリを使うアプリのローカルデータ配下)。
pub fn default_quarantine_dir() -> PathBuf {
    std::env::temp_dir().join("rs-smarttcp-quarantine")
}

/// Windows Security Center(`root/SecurityCenter2`、公式WMI名前空間)に
/// 何らかのウイルス対策ソフトが登録されているかどうかを確認する
/// (2026-08-11追加、ユーザー指示「無料版でも良いのでコンピューター
/// ウイルスとマルウェア対策のセキュリティソフトをインストールしま
/// しょう!と日本語と英語で表示する機能を付けて」への対応)。
/// **正直な開示**: 登録の有無のみを見ており、各製品のリアルタイム
/// 保護が実際に有効かどうか(`productState`のビット詳細)までは解析
/// していない——「1件も登録されていない」という明確なケースのみを
/// 検出し、インストールを促す。
pub fn has_registered_antivirus() -> bool {
    let script = "(Get-CimInstance -Namespace root/SecurityCenter2 -ClassName AntivirusProduct -ErrorAction SilentlyContinue | Measure-Object).Count";
    match Command::new("powershell").args(["-NoProfile", "-Command", script]).output() {
        Ok(out) => String::from_utf8_lossy(&out.stdout).trim().parse::<u32>().unwrap_or(0) > 0,
        Err(_) => false,
    }
}

/// セキュリティソフト未導入時に表示する、日英併記の推奨メッセージ。
pub fn install_security_software_recommendation() -> (String, String) {
    (
        "No antivirus/anti-malware security software was detected on this PC. Please install one — even a free edition is fine (e.g. ClamAV, KINGSOFT Internet Security, or Windows Defender).".to_string(),
        "このPCにコンピューターウイルス・マルウェア対策のセキュリティソフトが見つかりませんでした。無料版でも良いのでインストールしましょう!(例: ClamAV、キングソフト インターネットセキュリティ、Windows Defenderなど)".to_string(),
    )
}

/// 指定したファイルが、Windowsの「Mark of the Web」(インターネットから
/// ダウンロードされたことを示す代替データストリーム
/// `<ファイル名>:Zone.Identifier`)を持つかどうかを判定する。
/// ブラウザ・メールクライアント等が実際に付与する、実在するWindowsの
/// セキュリティ機構(SmartScreenの警告表示等が利用するのと同じ仕組み)。
pub fn has_mark_of_the_web(path: &Path) -> bool {
    let ads_path = format!("{}:Zone.Identifier", path.display());
    Path::new(&ads_path).exists()
}

/// 実行前にスキャン・確認が必要かどうかを判定する(呼び出し側が実際に
/// 実行をブロックするための判断材料として使う関数——**本クレート自身が
/// OSレベルで実行を強制的に阻止する機能は持たない**、正直な開示)。
pub fn should_block_execution_until_scanned(path: &Path) -> bool {
    has_mark_of_the_web(path)
}

/// 「Mark of the Web」を解除する(=Windowsのファイルのプロパティに
/// ある「ブロックの解除」チェックボックスと全く同じ、実在するWindows
/// 標準の仕組み——`Zone.Identifier`代替データストリームを削除する
/// だけ)。**正直な開示・重要な安全設計**: この関数は**スキャン結果が
/// `Clean`または`ThreatRemoved`の場合にのみ**呼び出す前提であり、
/// `verify_and_unblock`経由での利用を推奨する。ファイルを代わりに
/// 起動する機能は一切持たない——ブロックを解除した後、実際に開くか
/// どうかは常にユーザー自身の操作(ダブルクリック等)に委ねる設計
/// (ユーザー指示「ダウンロードしたファイルを自動実行する」機能・
/// 「AIが自動判定して必要なら自動実行する」機能は、スキャンで安全と
/// 判定されても残るゼロデイ・スキャン回避のリスクをそのまま自動実行に
/// つなげてしまうため、意図的に実装しないという合意に基づく)。
fn remove_mark_of_the_web(path: &Path) -> bool {
    let ads_path = format!("{}:Zone.Identifier", path.display());
    std::fs::remove_file(&ads_path).is_ok()
}

#[derive(Debug, Clone)]
pub struct VerifyAndUnblockResult {
    pub scan: ScanResult,
    /// スキャンが`Clean`または`ThreatRemoved`だった場合のみ`true`になり
    /// うる。ブロック解除に成功したかどうか(元々ブロックされていな
    /// かった場合も`false`——実際に解除処理が効いたかどうかを正直に
    /// 反映する)。
    pub unblocked: bool,
}

/// スキャン→(安全と確認できた場合のみ)Mark of the Web解除、という
/// 一連の流れを行う。**ファイルを実行する処理は一切含まない**——
/// あくまで「安全と確認できたファイルについて、以後ダブルクリックで
/// 開いてもSmartScreen等の警告が出ないようにする」ところまでに留める。
pub fn verify_and_unblock(backend: ScannerBackend, path: &Path, quarantine_dir: &Path) -> VerifyAndUnblockResult {
    let scan = scan_file(backend, path, quarantine_dir);
    let unblocked = match scan.outcome {
        ScanOutcome::Clean => remove_mark_of_the_web(path),
        ScanOutcome::ThreatRemoved => false, // 元のパスにファイルはもう存在しない(隔離済み)。
        _ => false,
    };
    VerifyAndUnblockResult { scan, unblocked }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_zero_means_clean() {
        let result = interpret_scan_result(Some(0), "", false);
        assert_eq!(result.outcome, ScanOutcome::Clean);
        assert!(result.message_ja.contains("安全"));
    }

    #[test]
    fn exit_code_one_with_quarantine_means_threat_removed() {
        let result = interpret_scan_result(Some(1), "Eicar-Test-Signature FOUND", true);
        assert_eq!(result.outcome, ScanOutcome::ThreatRemoved);
        assert_eq!(result.message_ja, "コンピューターウイルスを取り除きましたので、ファイルは安全です。");
    }

    #[test]
    fn exit_code_one_without_quarantine_means_threat_not_removed() {
        let result = interpret_scan_result(Some(1), "Eicar-Test-Signature FOUND", false);
        assert_eq!(result.outcome, ScanOutcome::ThreatFoundNotRemoved);
    }

    #[test]
    fn exit_code_two_or_missing_means_scanner_error() {
        assert_eq!(interpret_scan_result(Some(2), "", false).outcome, ScanOutcome::ScannerUnavailable);
        assert_eq!(interpret_scan_result(None, "", false).outcome, ScanOutcome::ScannerUnavailable);
    }

    #[test]
    fn nonexistent_file_has_no_mark_of_the_web() {
        assert!(!has_mark_of_the_web(Path::new("Z:\\definitely\\does\\not\\exist.txt")));
    }

    #[test]
    fn remove_mark_of_the_web_only_attempted_when_clean_or_no_op_otherwise() {
        // scan_fileはClamAV未インストール環境ではScannerUnavailableを
        // 返す(このテスト環境の実際の状態)——その場合、決して
        // unblocked: trueにならないことを確認する(黙って安全側へ
        // 倒れることの検証、実行を許可する誤判定が起きないこと)。
        let temp_dir = std::env::temp_dir().join(format!("rs-smarttcp-unblock-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file = temp_dir.join("sample.txt");
        std::fs::write(&file, "hello").unwrap();

        let result = verify_and_unblock(ScannerBackend::ClamAv, &file, &default_quarantine_dir());
        assert!(!result.unblocked, "must never unblock when the scan outcome wasn't Clean");
        assert_eq!(result.scan.outcome, ScanOutcome::ScannerUnavailable);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
