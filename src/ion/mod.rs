/*
 * The following license applies to this file, which was initially
 * derived from the files `js/src/jit/BacktrackingAllocator.h` and
 * `js/src/jit/BacktrackingAllocator.cpp` in Mozilla Firefox:
 *
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 *
 * Since the initial port, the design has been substantially evolved
 * and optimized.
 */

//! Backtracking register allocator. See doc/DESIGN.md for details of
//! its design.

use crate::cfg::CFGInfo;
use crate::{Function, MachineEnv, Output, PReg, ProgPoint, RegAllocError, RegClass};
use std::collections::HashMap;

pub(crate) mod data_structures;
pub use data_structures::Stats;
use data_structures::*;
pub(crate) mod reg_traversal;
use reg_traversal::*;
pub(crate) mod requirement;
use requirement::*;
pub(crate) mod redundant_moves;
use redundant_moves::*;
pub(crate) mod liveranges;
use liveranges::*;
pub(crate) mod merge;
pub(crate) mod process;
use process::*;
pub(crate) mod dump;
pub(crate) mod moves;
pub(crate) mod spill;
pub(crate) mod stackmap;

impl<'a, F: Function> Env<'a, F> {
    pub(crate) fn new(
        func: &'a F,
        env: &'a MachineEnv,
        cfginfo: CFGInfo,
        annotations_enabled: bool,
    ) -> Self {
        let n = func.num_insts();
        Self {
            func,
            env,
            cfginfo,

            liveins: Vec::with_capacity(func.num_blocks()),
            liveouts: Vec::with_capacity(func.num_blocks()),
            blockparam_outs: vec![],
            blockparam_ins: vec![],
            blockparam_allocs: vec![],
            bundles: Vec::with_capacity(n),
            ranges: Vec::with_capacity(4 * n),
            spillsets: Vec::with_capacity(n),
            vregs: Vec::with_capacity(n),
            vreg_regs: Vec::with_capacity(n),
            pregs: vec![],
            allocation_queue: PrioQueue::new(),
            safepoints: vec![],
            safepoints_per_vreg: HashMap::new(),
            spilled_bundles: vec![],
            spillslots: vec![],
            slots_by_size: vec![],
            allocated_bundle_count: 0,

            extra_spillslot: vec![None, None],

            prog_move_srcs: Vec::with_capacity(n / 2),
            prog_move_dsts: Vec::with_capacity(n / 2),
            prog_move_merges: Vec::with_capacity(n / 2),

            multi_fixed_reg_fixups: vec![],
            inserted_moves: vec![],
            edits: Vec::with_capacity(n),
            allocs: Vec::with_capacity(4 * n),
            inst_alloc_offsets: vec![],
            num_spillslots: 0,
            safepoint_slots: vec![],

            stats: Stats::default(),

            debug_annotations: std::collections::HashMap::new(),
            annotations_enabled,
        }
    }

    pub(crate) fn init(&mut self) -> Result<(), RegAllocError> {
        self.create_pregs_and_vregs();
        self.compute_liveness()?;
        self.merge_vreg_bundles();
        self.queue_bundles();
        if log::log_enabled!(log::Level::Trace) {
            self.dump_state();
        }
        Ok(())
    }

    pub(crate) fn run(&mut self) -> Result<(), RegAllocError> {
        self.process_bundles()?;
        self.try_allocating_regs_for_spilled_bundles();
        self.allocate_spillslots();
        self.apply_allocations_and_insert_moves();
        self.resolve_inserted_moves();
        self.compute_stackmaps();
        Ok(())
    }
}

pub fn run<F: Function>(
    func: &F,
    mach_env: &MachineEnv,
    enable_annotations: bool,
) -> Result<Output, RegAllocError> {
    let cfginfo = CFGInfo::new(func)?;

    let mut env = Env::new(func, mach_env, cfginfo, enable_annotations);
    env.init()?;

    env.run()?;

    if enable_annotations {
        env.dump_results();
    }

    Ok(Output {
        edits: env
            .edits
            .into_iter()
            .map(|(pos, _, edit)| (ProgPoint::from_index(pos), edit))
            .collect(),
        allocs: env.allocs,
        inst_alloc_offsets: env.inst_alloc_offsets,
        num_spillslots: env.num_spillslots as usize,
        debug_locations: vec![],
        safepoint_slots: env.safepoint_slots,
        stats: env.stats,
    })
}
