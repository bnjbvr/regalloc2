/*
 * Released under the terms of the Apache 2.0 license with LLVM
 * exception. See `LICENSE` for details.
 */

//! Lightweight CFG analyses.

use crate::{domtree, postorder, Block, Function, Inst, OperandKind, ProgPoint, RegAllocError};
use smallvec::{smallvec, SmallVec};

#[derive(Clone, Debug)]
pub struct CFGInfo {
    /// Postorder traversal of blocks.
    pub postorder: Vec<Block>,
    /// Domtree parents, indexed by block.
    pub domtree: Vec<Block>,
    /// For each instruction, the block it belongs to.
    pub insn_block: Vec<Block>,
    /// For each vreg, the instruction that defines it, if any.
    pub vreg_def_inst: Vec<Inst>,
    /// For each vreg, the block that defines it as a blockparam, if
    /// any. (Every vreg must have a valid entry in either
    /// `vreg_def_inst` or `vreg_def_blockparam`.)
    pub vreg_def_blockparam: Vec<(Block, u32)>,
    /// For each block, the first instruction.
    pub block_entry: Vec<ProgPoint>,
    /// For each block, the last instruction.
    pub block_exit: Vec<ProgPoint>,
    /// For each block, what is its position in its successor's preds,
    /// if it has a single successor?
    ///
    /// (Because we require split critical edges, we always either have a single
    /// successor (which itself may have multiple preds), or we have multiple
    /// successors but each successor itself has only one pred; so we can store
    /// just one value per block and always know any block's position in its
    /// successors' preds lists.)
    pub pred_pos: Vec<usize>,
    /// For each block, what is the approximate loop depth?
    ///
    /// This measure is fully precise iff the input CFG is reducible
    /// and blocks are in RPO, so that loop backedges are precisely
    /// those whose block target indices are less than their source
    /// indices. Otherwise, it will be approximate, but should still
    /// be usable for heuristic purposes.
    pub approx_loop_depth: Vec<u32>,
    /// What are the loop-depth transition points?
    pub loop_transition_points: Vec<ProgPoint>,
}

impl CFGInfo {
    pub fn new<F: Function>(f: &F) -> Result<CFGInfo, RegAllocError> {
        let postorder =
            postorder::calculate(f.blocks(), f.entry_block(), |block| f.block_succs(block));
        let domtree = domtree::calculate(
            f.blocks(),
            |block| f.block_preds(block),
            &postorder[..],
            f.entry_block(),
        );
        let mut insn_block = vec![Block::invalid(); f.insts()];
        let mut vreg_def_inst = vec![Inst::invalid(); f.num_vregs()];
        let mut vreg_def_blockparam = vec![(Block::invalid(), 0); f.num_vregs()];
        let mut block_entry = vec![ProgPoint::before(Inst::invalid()); f.blocks()];
        let mut block_exit = vec![ProgPoint::before(Inst::invalid()); f.blocks()];
        let mut pred_pos = vec![0; f.blocks()];
        let mut backedge_in = vec![0; f.blocks()];
        let mut backedge_out = vec![0; f.blocks()];

        for block in 0..f.blocks() {
            let block = Block::new(block);
            for (i, param) in f.block_params(block).iter().enumerate() {
                vreg_def_blockparam[param.vreg()] = (block, i as u32);
            }
            for inst in f.block_insns(block).iter() {
                insn_block[inst.index()] = block;
                for operand in f.inst_operands(inst) {
                    match operand.kind() {
                        OperandKind::Def => {
                            vreg_def_inst[operand.vreg().vreg()] = inst;
                        }
                        _ => {}
                    }
                }
            }
            block_entry[block.index()] = ProgPoint::before(f.block_insns(block).first());
            block_exit[block.index()] = ProgPoint::after(f.block_insns(block).last());

            // Check critical edge condition: if there is more than
            // one predecessor, each must have only one successor
            // (this block).
            let preds = f.block_preds(block).len() + if block == f.entry_block() { 1 } else { 0 };
            if preds > 1 {
                for (i, &pred) in f.block_preds(block).iter().enumerate() {
                    let succs = f.block_succs(pred).len();
                    if succs > 1 {
                        return Err(RegAllocError::CritEdge(pred, block));
                    }
                    pred_pos[pred.index()] = i;
                }
            }

            // Check branch-arg condition: if any successors have more
            // than one predecessor (given above, there will only be
            // one such successor), then the last instruction of this
            // block (the branch) cannot have any args other than the
            // blockparams.
            let mut require_no_branch_args = false;
            for &succ in f.block_succs(block) {
                let preds = f.block_preds(succ).len() + if succ == f.entry_block() { 1 } else { 0 };
                if preds > 1 {
                    require_no_branch_args = true;
                }
            }
            if require_no_branch_args {
                let last = f.block_insns(block).last();
                if f.branch_blockparam_arg_offset(block, last) > 0 {
                    return Err(RegAllocError::DisallowedBranchArg(last));
                }
            }

            for &succ in f.block_succs(block) {
                if succ.index() <= block.index() {
                    backedge_in[succ.index()] += 1;
                    backedge_out[block.index()] += 1;
                }
            }
        }

        let mut approx_loop_depth = vec![];
        let mut backedge_stack: SmallVec<[usize; 4]> = smallvec![];
        let mut cur_depth = 0;
        for block in 0..f.blocks() {
            if backedge_in[block] > 0 {
                cur_depth += 1;
                backedge_stack.push(backedge_in[block]);
            }

            approx_loop_depth.push(cur_depth);

            while backedge_stack.len() > 0 && backedge_out[block] > 0 {
                backedge_out[block] -= 1;
                *backedge_stack.last_mut().unwrap() -= 1;
                if *backedge_stack.last().unwrap() == 0 {
                    cur_depth -= 1;
                    backedge_stack.pop();
                }
            }
        }

        let mut loop_transition_points = vec![];
        let mut last_depth = approx_loop_depth[0];
        for (block, &depth) in approx_loop_depth.iter().enumerate() {
            if depth > last_depth {
                loop_transition_points.push(block_entry[block]);
            }
            last_depth = depth;
        }

        Ok(CFGInfo {
            postorder,
            domtree,
            insn_block,
            vreg_def_inst,
            vreg_def_blockparam,
            block_entry,
            block_exit,
            pred_pos,
            approx_loop_depth,
            loop_transition_points,
        })
    }

    pub fn dominates(&self, a: Block, b: Block) -> bool {
        domtree::dominates(&self.domtree[..], a, b)
    }

    /// Return the position of this block in its successor's predecessor list.
    ///
    /// Because the CFG must have split critical edges, we actually do not need
    /// to know *which* successor: if there is more than one, then each
    /// successor has only one predecessor (that's this block), so the answer is
    /// `0` no matter which successor we are considering.
    pub fn pred_position(&self, block: Block) -> usize {
        self.pred_pos[block.index()]
    }
}
