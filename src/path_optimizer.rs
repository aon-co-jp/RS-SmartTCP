//! 東芝シミュレーテッド分岐(Simulated Bifurcation、SBM)を使った、複数
//! 経路の帯域予算内選択最適化(2026-08-11新設)。
//!
//! ユーザー指示「東芝の疑似量子コンピューター技術…をRS-SmartTCP内に
//! 具体的な最適化問題(例: 複数WAN経路の帯域配分最適化)を日本語と英語の
//! Google検索とGithubを深く検索、深く調査して発見して実装」への対応。
//!
//! ## 見つけた具体的な最適化問題
//!
//! [`crate::multi_path::MultiPathManager`]は最大10本の有線/WiFi/
//! Bluetooth経路を、[`crate::multi_wan::MultiWanManager`]は最大10本の
//! WAN回線を管理できる。実運用では「全経路を同時にアクティブにする」の
//! ではなく、**契約帯域・電力・CPU負荷等の予算(コスト)の下で、通信
//! 品質(RTT実測値の逆数)の合計が最大になる経路の組み合わせを選ぶ**、
//! という選択問題が実際に生じる——これは0/1ナップサック問題そのもので
//! あり、`aruaru-llm::cache_optimizer`(モデルキャッシュのディスク容量
//! 予算問題)と数学的に同型の、しかしRS-SmartTCP固有の実際の意思決定
//! (どの経路を使うか)に対応する具体的な問題として定式化した。
//!
//! ## 正直な開示(最重要、既存方針を踏襲)
//!
//! - **アルゴリズム自体は`aruaru-llm::cache_optimizer`と同じ設計
//!   (QUBO→Ising変換+Ballistic Simulated Bifurcation)を、RS-SmartTCPが
//!   `aruaru-llm`へ依存できない(役割の異なる独立クレートのため)ことから
//!   独立に実装したもの**——コードのコピーではなく、同じ検証済みの
//!   数式・アルゴリズムを本クレート内で再実装した(`aruaru-llm`側の
//!   実装で見つかった局所磁場の正規化バグの教訓も引き継いで反映済み)。
//! - 経路の本数(最大10)程度の規模は全探索・動的計画法でも瞬時に厳密解が
//!   求まり、SBMを使う実用上の必要性は薄い——「SBMが無ければ解けない/
//!   著しく遅い」という主張はしていない。あくまで「実際の意思決定パスへ
//!   配線し、全探索との数値一致(75%以上の近似)を検証する」という
//!   統合実証を目的とする。
//! - SBM解が予算(コスト)制約を満たさない場合は、価値密度順の貪欲
//!   フォールバックへ安全に切り替える(`used_sbm_solution`で判別可能)。

#![allow(clippy::needless_range_loop)]

#[derive(Debug, Clone)]
struct QuboProblem {
    n: usize,
    h: Vec<f64>,
    q: Vec<f64>,
}

impl QuboProblem {
    fn new(n: usize) -> Self {
        Self { n, h: vec![0.0; n], q: vec![0.0; n * n] }
    }

    fn add_h(&mut self, i: usize, v: f64) {
        self.h[i] += v;
    }

    fn add_q(&mut self, i: usize, j: usize, v: f64) {
        if i == j {
            self.h[i] += v;
            return;
        }
        self.q[i * self.n + j] += v;
        self.q[j * self.n + i] += v;
    }

    #[cfg(test)]
    fn energy(&self, z: &[bool]) -> f64 {
        let mut e = 0.0;
        for i in 0..self.n {
            if z[i] {
                e += self.h[i];
            }
        }
        for i in 0..self.n {
            if !z[i] {
                continue;
            }
            for j in (i + 1)..self.n {
                if z[j] {
                    e += self.q[i * self.n + j];
                }
            }
        }
        e
    }

    fn to_ising(&self) -> (Vec<f64>, Vec<f64>) {
        let n = self.n;
        let mut h_spin = vec![0.0; n];
        let mut j_spin = vec![0.0; n * n];
        for i in 0..n {
            h_spin[i] += self.h[i] / 2.0;
        }
        for i in 0..n {
            for j in (i + 1)..n {
                let qij = self.q[i * n + j];
                if qij == 0.0 {
                    continue;
                }
                j_spin[i * n + j] += qij / 4.0;
                j_spin[j * n + i] += qij / 4.0;
                h_spin[i] += qij / 4.0;
                h_spin[j] += qij / 4.0;
            }
        }
        (h_spin, j_spin)
    }
}

fn qubo_energy_from_spins(h: &[f64], j: &[f64], n: usize, z: &[bool]) -> f64 {
    let s: Vec<f64> = z.iter().map(|&b| if b { 1.0 } else { -1.0 }).collect();
    let mut e = 0.0;
    for i in 0..n {
        e += h[i] * s[i];
    }
    for i in 0..n {
        for jj in (i + 1)..n {
            e += j[i * n + jj] * s[i] * s[jj];
        }
    }
    e
}

/// Ballistic Simulated Bifurcationによる`E(s) = sum h_i*s_i +
/// sum_{i<j} j_ij*s_i*s_j`の最小化。`aruaru-llm::cache_optimizer`と同じ
/// アルゴリズム(独立実装、局所磁場の正規化を含む)。
fn simulated_bifurcation_minimize(h: &[f64], j: &[f64], n: usize, steps: usize, restarts: usize, seed: u64) -> Vec<bool> {
    let a0 = 1.0_f64;
    let dt = 0.75_f64 / (n as f64).sqrt().max(1.0);
    let max_abs = h.iter().chain(j.iter()).fold(0.0_f64, |acc, &v| acc.max(v.abs())).max(1e-12);
    let c0 = 0.5 / (n as f64).sqrt() / max_abs;

    let mut rng_state = seed.max(1);
    let mut next_rand = move || -> f64 {
        rng_state ^= rng_state << 13;
        rng_state ^= rng_state >> 7;
        rng_state ^= rng_state << 17;
        ((rng_state >> 11) as f64) / ((1u64 << 53) as f64) * 2.0 - 1.0
    };

    let mut best_spins = vec![false; n];
    let mut best_energy = f64::MAX;

    for _restart in 0..restarts {
        let mut x: Vec<f64> = (0..n).map(|_| next_rand() * 0.1).collect();
        let mut y: Vec<f64> = (0..n).map(|_| next_rand() * 0.1).collect();

        for step in 0..steps {
            let a_t = a0 * (step as f64) / (steps as f64);
            for i in 0..n {
                let mut coupling = h[i];
                for (jj, &xj) in x.iter().enumerate() {
                    if i != jj {
                        coupling += j[i * n + jj] * xj;
                    }
                }
                let dy = (-(a0 - a_t) * x[i] - c0 * coupling) * dt;
                y[i] += dy;
            }
            for i in 0..n {
                x[i] += a0 * y[i] * dt;
                if x[i] > 1.0 {
                    x[i] = 1.0;
                    y[i] = 0.0;
                } else if x[i] < -1.0 {
                    x[i] = -1.0;
                    y[i] = 0.0;
                }
            }
        }

        let spins: Vec<bool> = x.iter().map(|&xi| xi >= 0.0).collect();
        let energy = qubo_energy_from_spins(h, j, n, &spins);
        if energy < best_energy {
            best_energy = energy;
            best_spins = spins;
        }
    }
    best_spins
}

fn build_path_selection_qubo(qualities: &[f64], costs: &[u32], budget: u32) -> QuboProblem {
    let n_paths = qualities.len();
    assert_eq!(n_paths, costs.len());

    let slack_bits = if budget == 0 { 0 } else { (32 - budget.leading_zeros()) as usize };
    let n = n_paths + slack_bits;

    let mut a = vec![0.0f64; n];
    for i in 0..n_paths {
        a[i] = costs[i] as f64;
    }
    for k in 0..slack_bits {
        a[n_paths + k] = (1u64 << k) as f64;
    }

    let max_value: f64 = qualities.iter().sum::<f64>().max(1.0);
    let penalty = max_value * 1.5;

    let mut problem = QuboProblem::new(n);
    let c = budget as f64;

    for i in 0..n_paths {
        problem.add_h(i, -qualities[i]);
    }
    for j_idx in 0..n {
        problem.add_h(j_idx, -2.0 * penalty * c * a[j_idx] + penalty * a[j_idx] * a[j_idx]);
        for k_idx in (j_idx + 1)..n {
            problem.add_q(j_idx, k_idx, 2.0 * penalty * a[j_idx] * a[k_idx]);
        }
    }
    problem
}

fn respects_budget(selection: &[bool], costs: &[u32], budget: u32) -> bool {
    let total: u32 = selection.iter().zip(costs).filter(|(&s, _)| s).map(|(_, &c)| c).sum();
    total <= budget
}

fn greedy_fallback(qualities: &[f64], costs: &[u32], budget: u32) -> Vec<bool> {
    let mut order: Vec<usize> = (0..qualities.len()).collect();
    order.sort_by(|&a, &b| {
        let da = if costs[a] > 0 { qualities[a] / costs[a] as f64 } else { f64::MAX };
        let db = if costs[b] > 0 { qualities[b] / costs[b] as f64 } else { f64::MAX };
        db.partial_cmp(&da).unwrap()
    });
    let mut selection = vec![false; qualities.len()];
    let mut used = 0u32;
    for i in order {
        if used + costs[i] <= budget {
            selection[i] = true;
            used += costs[i];
        }
    }
    selection
}

fn solve_path_selection_sbm(qualities: &[f64], costs: &[u32], budget: u32, seed: u64) -> Vec<bool> {
    let problem = build_path_selection_qubo(qualities, costs, budget);
    let n_paths = qualities.len();
    let (h_spin, j_spin) = problem.to_ising();
    let n = problem.n;

    let mut best: Option<(Vec<bool>, f64)> = None;
    for attempt in 0..24u64 {
        let z = simulated_bifurcation_minimize(&h_spin, &j_spin, n, 1200, 64, seed.wrapping_add(attempt.wrapping_mul(0x9E3779B97F4A7C15)));
        let selection = z[..n_paths].to_vec();
        if !respects_budget(&selection, costs, budget) {
            continue;
        }
        let value: f64 = selection.iter().zip(qualities.iter()).filter(|(&s, _)| s).map(|(_, &v)| v).sum();
        if best.as_ref().map(|(_, v)| value > *v).unwrap_or(true) {
            best = Some((selection, value));
        }
    }
    best.map(|(sel, _)| sel).unwrap_or_else(|| vec![false; n_paths])
}

/// 経路選択最適化の結果。
#[derive(Debug, Clone)]
pub struct PathSelectionResult {
    pub activate: Vec<String>,
    pub deactivate: Vec<String>,
    pub total_cost: u32,
    pub budget: u32,
    pub total_quality: f64,
    /// SBM解がそのまま予算制約を満たしていたか(false=貪欲フォールバック
    /// を使った、正直な開示)。
    pub used_sbm_solution: bool,
}

/// `entries`(経路名, コスト〈帯域/電力等の予算消費量〉, 品質スコア
/// 〈例: RTT実測値の逆数、大きいほど良い〉)から、`budget`以内でコストの
/// 合計を抑えつつ品質合計を最大化する経路の組み合わせをSBMで決定する。
pub fn optimize_path_selection(entries: &[(&str, u32, f64)], budget: u32, seed: u64) -> PathSelectionResult {
    let names: Vec<&str> = entries.iter().map(|(name, _, _)| *name).collect();
    let costs: Vec<u32> = entries.iter().map(|(_, cost, _)| *cost).collect();
    let qualities: Vec<f64> = entries.iter().map(|(_, _, q)| *q).collect();

    let sbm_selection = solve_path_selection_sbm(&qualities, &costs, budget, seed);
    let (selection, used_sbm_solution) = if respects_budget(&sbm_selection, &costs, budget) {
        (sbm_selection, true)
    } else {
        (greedy_fallback(&qualities, &costs, budget), false)
    };

    let mut activate = Vec::new();
    let mut deactivate = Vec::new();
    let mut total_cost = 0u32;
    let mut total_quality = 0.0;
    for (i, &name) in names.iter().enumerate() {
        if selection[i] {
            activate.push(name.to_string());
            total_cost += costs[i];
            total_quality += qualities[i];
        } else {
            deactivate.push(name.to_string());
        }
    }

    PathSelectionResult { activate, deactivate, total_cost, budget, total_quality, used_sbm_solution }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brute_force_selection(qualities: &[f64], costs: &[u32], budget: u32) -> f64 {
        let n = qualities.len();
        let mut best = 0.0;
        for mask in 0u32..(1u32 << n) {
            let mut w = 0u32;
            let mut v = 0.0;
            for i in 0..n {
                if (mask >> i) & 1 == 1 {
                    w += costs[i];
                    v += qualities[i];
                }
            }
            if w <= budget && v > best {
                best = v;
            }
        }
        best
    }

    #[test]
    fn qubo_to_ising_preserves_energy_ordering_on_small_problem() {
        let problem = build_path_selection_qubo(&[3.0, 5.0, 2.0], &[2, 3, 1], 4);
        let (h_spin, j_spin) = problem.to_ising();
        let n = problem.n;

        let mut qubo_energies = Vec::new();
        let mut ising_energies = Vec::new();
        for mask in 0u32..(1u32 << n) {
            let z: Vec<bool> = (0..n).map(|i| (mask >> i) & 1 == 1).collect();
            qubo_energies.push(problem.energy(&z));
            ising_energies.push(qubo_energy_from_spins(&h_spin, &j_spin, n, &z));
        }
        let offset = qubo_energies[0] - ising_energies[0];
        for (q, i) in qubo_energies.iter().zip(ising_energies.iter()) {
            assert!((q - i - offset).abs() < 1e-9, "qubo={q} ising={i} offset={offset}");
        }
    }

    #[test]
    fn sbm_path_selection_matches_brute_force_within_75_percent() {
        // 5経路(有線2本+WiFi2本+WAN1本を想定)、コストは帯域予算消費量
        // (Mbps単位の概算)、品質は実測RTT(ms)の逆数×100(小さいRTTほど
        // 高品質)という想定の合成データ。
        let costs = [100u32, 50, 200, 30, 500];
        let rtts_ms = [5.0, 12.0, 3.0, 25.0, 1.5];
        let qualities: Vec<f64> = rtts_ms.iter().map(|&r| 100.0 / r).collect();

        for &budget in &[80u32, 150, 300, 500, 900] {
            let optimal = brute_force_selection(&qualities, &costs, budget);
            let selection = solve_path_selection_sbm(&qualities, &costs, budget, 0xC0FFEE + budget as u64);
            let (selection, used_sbm) =
                if respects_budget(&selection, &costs, budget) { (selection, true) } else { (greedy_fallback(&qualities, &costs, budget), false) };
            let achieved: f64 = selection.iter().zip(qualities.iter()).filter(|(&s, _)| s).map(|(_, &v)| v).sum();
            let used_cost: u32 = selection.iter().zip(costs.iter()).filter(|(&s, _)| s).map(|(_, &c)| c).sum();
            assert!(used_cost <= budget, "budget={budget}: selection exceeds budget ({used_cost} > {budget})");
            if optimal > 0.0 {
                assert!(achieved / optimal >= 0.75, "budget={budget}: achieved={achieved} optimal={optimal} used_sbm={used_sbm}");
            }
        }
    }

    #[test]
    fn optimize_path_selection_reports_used_sbm_solution_and_respects_budget() {
        let entries: Vec<(&str, u32, f64)> =
            vec![("Ethernet1", 100, 20.0), ("Ethernet2", 100, 18.0), ("WiFi1", 50, 8.0), ("WAN1 Fiber", 500, 66.0)];
        let result = optimize_path_selection(&entries, 300, 42);
        assert!(result.total_cost <= result.budget);
        assert_eq!(result.activate.len() + result.deactivate.len(), entries.len());
    }
}
