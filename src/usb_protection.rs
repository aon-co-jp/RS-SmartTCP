//! USBメモリ等のリムーバブルドライブ挿入時の保護(2026-08-11新設)。
//!
//! ユーザー指示「セキュリティソフトの機能実装としては、USBスティック
//! メモリーなどを刺した瞬間にコンピューターウイルスが侵入したり」への
//! 対応(自動実行〈autorun.inf〉経由の感染経路への対策+挿入時スキャン)。
//!
//! ## 正直な開示(最重要)
//!
//! - **Windows Vista/7以降、USBリムーバブルドライブのautorun.inf実行は
//!   OS標準でもとから無効化されている**(自動実行が働くのはCD/DVD等の
//!   光学メディアのみ、という既知の仕様変更)。とはいえ、悪意ある
//!   `autorun.inf`ファイル自体がドライブに存在すること自体は攻撃の
//!   痕跡・別経路(ユーザーが手動でダブルクリックしてしまう等)の
//!   リスクであり続けるため、本モジュールは実際に見つけて隔離する。
//! - **削除ではなくリネームによる無害化**: `autorun.inf`を完全削除
//!   するのではなく`autorun.inf.quarantined`へリネームする(可逆的、
//!   誤検知だった場合に戻せる、既存の`download_protection`モジュールの
//!   「隔離>削除」という設計方針と一貫)。
//! - **ドライブの実際のスキャンは`download_protection`モジュールの
//!   実装(ClamAV/KINGSOFT)をそのまま再利用する**(独自のウイルス検出
//!   エンジンを別途実装しない)。
//! - **USB挿入の「常時監視」自体は実装していない**: Windowsの
//!   デバイス挿入イベント(`WM_DEVICECHANGE`)をリアルタイムに監視する
//!   にはウィンドウメッセージループへのフックが必要で、このクレートの
//!   `std::net`ベースのGUI例(`status_gui.rs`)の範囲を超える——本
//!   モジュールが提供するのは「現在挿さっているリムーバブルドライブを
//!   検出し、要求に応じて保護処理を行う」関数群であり、呼び出し側
//!   (実際のアプリ)が定期的に呼ぶか、OSのイベント通知と組み合わせる
//!   ことを想定する。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::download_protection::{self, ScanResult, ScannerBackend};

/// 「刺した瞬間」を検知するための簡易ポーリング用ヘルパー。
///
/// 前述の通り本クレートは`WM_DEVICECHANGE`のようなOSイベントフックを
/// 持たないため、呼び出し側が数秒間隔で本関数を呼ぶことで「挿した瞬間」
/// に近い検知を実現する想定(例: `status_gui.rs`や実アプリのバック
/// グラウンドタイマーから定期呼び出し)。`seen`には前回呼び出し時点の
/// ドライブ集合を渡し、本関数が最新の状態に更新した上で、新規に増えた
/// ドライブだけを返す。
pub fn poll_new_drives(seen: &mut HashSet<PathBuf>) -> Vec<PathBuf> {
    let current: HashSet<PathBuf> = list_removable_drives().into_iter().collect();
    let newly_inserted: Vec<PathBuf> = current.difference(seen).cloned().collect();
    *seen = current;
    newly_inserted
}

/// 現在接続されているリムーバブルドライブ(USBメモリ等)のドライブ
/// レター一覧を返す。PowerShellの`Win32_LogicalDisk`(`DriveType=2`が
/// リムーバブルメディアを表す、Microsoft公式ドキュメントに基づく標準
/// WMIクラス)を使う——外部Rustクレート非依存の既存方針を維持。
pub fn list_removable_drives() -> Vec<PathBuf> {
    let script = "Get-CimInstance -ClassName Win32_LogicalDisk -Filter \"DriveType=2\" | ForEach-Object { $_.DeviceID }";
    match Command::new("powershell").args(["-NoProfile", "-Command", script]).output() {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).map(|drive_letter| PathBuf::from(format!("{drive_letter}\\"))).collect()
        }
        Err(_) => Vec::new(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutorunCheckResult {
    pub found: bool,
    pub quarantined: bool,
}

/// ドライブ直下の`autorun.inf`を探し、見つかれば
/// `autorun.inf.quarantined`へリネームして無害化する(削除ではなく
/// リネーム、可逆的)。存在しない場合は`found: false`を正直に返す。
pub fn neutralize_autorun_inf(drive_root: &Path) -> AutorunCheckResult {
    let autorun_path = drive_root.join("autorun.inf");
    if !autorun_path.exists() {
        return AutorunCheckResult { found: false, quarantined: false };
    }
    let quarantined_path = drive_root.join("autorun.inf.quarantined");
    let quarantined = std::fs::rename(&autorun_path, &quarantined_path).is_ok();
    AutorunCheckResult { found: true, quarantined }
}

/// リムーバブルドライブを挿入時に保護する一括処理: (1) `autorun.inf`の
/// 無害化、(2) 選択したバックエンド([`ScannerBackend`])でドライブ全体を
/// スキャン。スキャン自体は[`crate::download_protection::scan_file`]の
/// 実装をそのまま再利用する(独自のウイルス検出ロジックは持たない)。
pub fn protect_drive(drive_root: &Path, backend: ScannerBackend) -> (AutorunCheckResult, ScanResult) {
    let autorun_result = neutralize_autorun_inf(drive_root);
    let quarantine_dir = download_protection::default_quarantine_dir();
    let scan_result = download_protection::scan_file(backend, drive_root, &quarantine_dir);
    (autorun_result, scan_result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutralize_autorun_inf_reports_not_found_when_absent() {
        let temp = std::env::temp_dir().join(format!("rs-smarttcp-usb-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp);
        let result = neutralize_autorun_inf(&temp);
        assert!(!result.found);
        assert!(!result.quarantined);
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn neutralize_autorun_inf_renames_existing_file_instead_of_deleting() {
        let temp = std::env::temp_dir().join(format!("rs-smarttcp-usb-test2-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp);
        std::fs::write(temp.join("autorun.inf"), "[autorun]\nopen=malware.exe").unwrap();

        let result = neutralize_autorun_inf(&temp);
        assert!(result.found);
        assert!(result.quarantined);
        assert!(!temp.join("autorun.inf").exists(), "original autorun.inf must be gone from its original name");
        assert!(temp.join("autorun.inf.quarantined").exists(), "content must be preserved under a quarantined name, not deleted");

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn poll_new_drives_reports_only_newly_appeared_drives_and_updates_seen_set() {
        let mut seen: HashSet<PathBuf> = HashSet::new();
        seen.insert(PathBuf::from(r"Z:\")); // 実在しないドライブを既知として先に登録

        let newly_inserted = poll_new_drives(&mut seen);

        // このテスト環境の実際のリムーバブルドライブ一覧が全て「新規」
        // として返ってくるはず(Z:はダミーの既知ドライブなので対象外)。
        assert!(!newly_inserted.contains(&PathBuf::from(r"Z:\")));
        // 呼び出し後、seenは実際のドライブ集合で更新されているはず。
        assert_eq!(seen, list_removable_drives().into_iter().collect::<HashSet<_>>());
    }
}
