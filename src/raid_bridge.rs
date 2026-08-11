//! RAID-Z2/Z3(RAID6/RAID7相当)パリティ計算のGPU/NPU高速化ブリッジ
//! (2026-08-11新設、ユーザー指示「RAID6のZ2やZ3対応でパリティチェックの
//! 高速化やnVME SSD」への対応)。
//!
//! ## 正直な開示(最重要)
//!
//! **本モジュールは新しいRAID実装をゼロから書いていない。**
//! `open-raid-z`(`open_raid_z_core::vdev::RaidZVdev`)に既に実在する
//! RAID-Z2/Z3(GF(2^8) Reed-SolomonによるP/Q〈Z2〉・P/Q/R〈Z3〉パリティ)+
//! `zfs_accel_hlsl`(D3D12/DirectML〈Windows〉・Vulkan Compute
//! 〈Linux/macOS/Android〉によるGPU/NPUアクセラレーション、自動検出+
//! CPUフォールバック)を、path依存でそのまま呼び出す薄いブリッジに
//! 留める(`dream-os-raid-bridge`と全く同じ設計方針、車輪の再発明を
//! 避ける既存エコシステム方針)。
//!
//! ## NVMe SSDについて(重要な制約)
//!
//! **`open_raid_z_core::block_device::BlockDevice`トレイトの実装は
//! 現時点で`FileBackedDevice`(通常ファイルへのループバック)のみであり、
//! 実NVMeデバイス(`/dev/nvme0n1`等への直接I/O、O_DIRECT・アライメント
//! 考慮)向けの実装は`open-raid-z`側にもこのブリッジ側にも存在しない**
//! (`dream-os-raid-bridge`の既存の正直な開示と同じ状況を2026-08-11
//! 時点で再確認)。`BlockDevice`トレイト自体は実ブロックデバイスにも
//! 対応できる抽象度で設計されているため、将来実装を追加すること自体は
//! 可能だが、今回は着手していない——「NVMe対応」を名乗る実装は本モジュール
//! には無く、ループバックファイルでの検証に留まる。

use std::path::Path;

#[cfg(test)]
use open_raid_z_core::block_device::BlockDevice;
use open_raid_z_core::block_device::FileBackedDevice;
use open_raid_z_core::vdev::{RaidLevel, RaidZVdev};
use zfs_accel_hlsl::device::{detect_best_accelerator, AccelDevice, AccelKind};

/// RAID-Z2/Z3のパリティ計算に使える最良のアクセラレータを検出する
/// (`open-raid-z`の既存フォールバック設計をそのまま利用、GPU/NPUが
/// 無ければ`AccelKind::CpuFallback`)。
pub fn detect_parity_accelerator() -> Option<AccelDevice> {
    detect_best_accelerator().ok()
}

pub fn is_cpu_fallback(accel: &AccelDevice) -> bool {
    accel.kind == AccelKind::CpuFallback
}

/// Z2(RAID6相当、パリティ2本)またはZ3(パリティ3本)の冗長構成を、
/// ループバックファイルベースの`RaidZVdev`として構築する。
/// **実NVMeデバイスではない**(冒頭の正直な開示参照)。
pub fn build_loopback_raidz(
    dir: &Path,
    level: RaidLevel,
    num_data_disks: usize,
    chunk_size: usize,
    stripe_count: u64,
    accel: Option<AccelDevice>,
) -> std::io::Result<RaidZVdev<FileBackedDevice>> {
    // Z2/Z3のパリティ本数はディスク総数に依存しないため、ダミー値で
    // `parity_count`を呼んでも正しい本数が得られる(Raid1のみ総数依存、
    // 本ブリッジはZ2/Z3専用のためこの呼び方で問題ない)。
    let total_disks = num_data_disks + level.parity_count(0);
    let disk_size = chunk_size as u64 * (stripe_count + 1);
    let devices: Vec<FileBackedDevice> = (0..total_disks)
        .map(|i| {
            let path = dir.join(format!("d{i}"));
            FileBackedDevice::create_fixed_size(&path, disk_size).map_err(|e| std::io::Error::other(e.to_string()))
        })
        .collect::<std::io::Result<_>>()?;

    let mut vdev = RaidZVdev::new(devices, level, chunk_size);
    if let Some(accel) = accel {
        vdev = vdev.with_accelerator(accel);
    }
    Ok(vdev)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rs-smarttcp-raidz-test-{tag}-{}", std::process::id()))
    }

    #[test]
    fn z2_write_read_roundtrip_and_self_heal_after_single_disk_corruption() {
        let dir = temp_dir("z2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let accel = detect_parity_accelerator();
        let mut vdev = build_loopback_raidz(&dir, RaidLevel::Z2, 4, 4096, 4, accel).unwrap();

        let stripe: Vec<u8> = (0..4096 * 4).map(|i| (i % 251) as u8).collect();
        vdev.write_stripe(0, &stripe).unwrap();
        let read_back = vdev.read_stripe(0).unwrap();
        assert_eq!(read_back, stripe);

        // 1台のディスクを直接壊しても、Z2(パリティ2本)なら復元できる。
        vdev.devices_mut()[1].write_at(0, &vec![0xFFu8; 4096]).unwrap();
        let (healed, mismatched) = vdev.read_stripe_with_report(0).unwrap();
        assert_eq!(healed, stripe, "single-disk corruption must be recoverable under Z2");
        assert_eq!(mismatched.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn z3_tolerates_two_simultaneous_disk_corruptions() {
        let dir = temp_dir("z3");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let accel = detect_parity_accelerator();
        let mut vdev = build_loopback_raidz(&dir, RaidLevel::Z3, 4, 4096, 4, accel).unwrap();

        let stripe: Vec<u8> = (0..4096 * 4).map(|i| (i * 7 % 251) as u8).collect();
        vdev.write_stripe(0, &stripe).unwrap();

        // Z3(パリティ3本)は2台までの同時破損から復元できるはず。
        vdev.devices_mut()[0].write_at(0, &vec![0xFFu8; 4096]).unwrap();
        vdev.devices_mut()[2].write_at(0, &vec![0xFFu8; 4096]).unwrap();
        let (healed, mismatched) = vdev.read_stripe_with_report(0).unwrap();
        assert_eq!(healed, stripe, "two-disk corruption must be recoverable under Z3");
        assert_eq!(mismatched.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
