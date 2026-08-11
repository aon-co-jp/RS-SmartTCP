//! PC起動時・定期実行を想定した自動メンテナンス(2026-08-11新設)。
//!
//! ユーザー指示「PC起動時や定期的な自動メンテナンスも行なうべきです
//! ね。セキュリティソフトに自動メンテナンス機能があると良いですね」
//! への対応。
//!
//! ## 正直な開示(最重要)
//!
//! - **本クレート自身はOSのタスクスケジューラ登録・常駐プロセス化を
//!   行わない**——「起動時に呼ばれる/定期的に呼ばれる」ようにする
//!   ことは、このクレートを使う側のアプリ(Windowsのタスクスケジューラ
//!   ・サービス化等)の責務である。本モジュールは「呼ばれたら実際に
//!   何をすべきか」の中身のみを提供する。
//! - **ウイルス定義の自動更新はClamAVの公式ツール
//!   (`freshclam`)をそのまま呼び出すだけ**——独自の定義データベースは
//!   実装していない。`freshclam`が見つからない場合は正直に
//!   `DefinitionsUpdateUnavailable`を返す(黙って「最新です」と
//!   偽らない)。
//! - **ここでもファイルの自動実行は一切行わない**(前段の合意通り)
//!   ——メンテナンスの内容は「セキュリティソフトの登録確認」「ウイルス
//!   定義の更新」に限定する。

use std::process::Command;

use crate::download_protection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefinitionsUpdateOutcome {
    /// `freshclam`を実際に実行し、正常終了した(最新化された、または
    /// 既に最新だった)。
    Updated,
    /// `freshclam`が見つからない、または実行に失敗した。
    DefinitionsUpdateUnavailable,
}

#[derive(Debug, Clone)]
pub struct MaintenanceReport {
    pub antivirus_registered: bool,
    pub definitions_update: DefinitionsUpdateOutcome,
    pub summary_en: String,
    pub summary_ja: String,
}

fn find_freshclam() -> Option<String> {
    if Command::new("freshclam").arg("--version").output().is_ok() {
        return Some("freshclam".to_string());
    }
    const CANDIDATES: &[&str] = &[r"C:\Program Files\ClamAV\freshclam.exe"];
    CANDIDATES.iter().find(|p| std::path::Path::new(p).exists()).map(|s| s.to_string())
}

/// ウイルス定義データベースの更新を試みる(ClamAV公式の`freshclam`を
/// そのまま実行、独自更新ロジックは無い)。
fn update_virus_definitions() -> DefinitionsUpdateOutcome {
    let Some(freshclam) = find_freshclam() else {
        return DefinitionsUpdateOutcome::DefinitionsUpdateUnavailable;
    };
    match Command::new(freshclam).output() {
        Ok(out) if out.status.success() => DefinitionsUpdateOutcome::Updated,
        _ => DefinitionsUpdateOutcome::DefinitionsUpdateUnavailable,
    }
}

/// PC起動時・定期実行(呼び出し側のスケジューラ/タスク管理から呼ばれる
/// 想定)のメンテナンス一式: (1) セキュリティソフトの登録確認、
/// (2) ClamAVのウイルス定義更新。ファイルの実行は一切行わない。
pub fn run_maintenance() -> MaintenanceReport {
    let antivirus_registered = download_protection::has_registered_antivirus();
    let definitions_update = update_virus_definitions();

    let mut summary_en = String::new();
    let mut summary_ja = String::new();

    if antivirus_registered {
        summary_en.push_str("Security software is registered with Windows Security Center. ");
        summary_ja.push_str("セキュリティソフトはWindows セキュリティセンターに登録されています。");
    } else {
        let (en, ja) = download_protection::install_security_software_recommendation();
        summary_en.push_str(&en);
        summary_en.push(' ');
        summary_ja.push_str(&ja);
    }

    match definitions_update {
        DefinitionsUpdateOutcome::Updated => {
            summary_en.push_str("ClamAV virus definitions are up to date.");
            summary_ja.push_str("ClamAVのウイルス定義は最新です。");
        }
        DefinitionsUpdateOutcome::DefinitionsUpdateUnavailable => {
            summary_en.push_str("ClamAV virus definitions could not be updated (freshclam not found or failed).");
            summary_ja.push_str("ClamAVのウイルス定義を更新できませんでした(freshclamが見つからない、または失敗しました)。");
        }
    }

    MaintenanceReport { antivirus_registered, definitions_update, summary_en, summary_ja }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_maintenance_never_panics_and_reports_a_definitions_outcome() {
        // このテスト環境にはfreshclamが無いため、正直に
        // DefinitionsUpdateUnavailableになることを確認する
        // (「更新済み」と偽らないことの検証)。
        let report = run_maintenance();
        assert_eq!(report.definitions_update, DefinitionsUpdateOutcome::DefinitionsUpdateUnavailable);
        assert!(!report.summary_en.is_empty());
        assert!(!report.summary_ja.is_empty());
    }
}
