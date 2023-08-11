use halo2_proofs::arithmetic::FieldExt;
use halo2_proofs::plonk::Advice;
use halo2_proofs::plonk::Column;
use halo2_proofs::plonk::Error;
use specs::itable::UnaryOp;
use specs::step::StepInfo;

use crate::circuits::utils::table_entry::EventTableWithMemoryInfo;
use crate::circuits::utils::Context;

use super::BitTableChip;
use super::BitTableOp;
use super::STEP_SIZE;

struct BitTableAssign {
    op: BitTableOp,
    left: u64,
    right: u64,
    result: u64,
}

fn filter_bit_table_entries(event_table: &EventTableWithMemoryInfo) -> Vec<BitTableAssign> {
    event_table
        .0
        .iter()
        .filter_map(|entry| match &entry.eentry.step_info {
            StepInfo::I32BinBitOp {
                class,
                left,
                right,
                value,
            } => Some(BitTableAssign {
                op: BitTableOp::BinaryBit(*class),
                left: *left as u32 as u64,
                right: *right as u32 as u64,
                result: *value as u32 as u64,
            }),

            StepInfo::I64BinBitOp {
                class,
                left,
                right,
                value,
            } => Some(BitTableAssign {
                op: BitTableOp::BinaryBit(*class),
                left: *left as u64,
                right: *right as u64,
                result: *value as u64,
            }),

            StepInfo::UnaryOp {
                class: UnaryOp::Popcnt,
                operand,
                ..
            } => Some(BitTableAssign {
                op: BitTableOp::Popcnt,
                left: *operand,
                right: 0,
                result: *operand, // Compute decomposed result in assignment
            }),

            _ => None,
        })
        .collect::<Vec<_>>()
}

impl<F: FieldExt> BitTableChip<F> {
    fn init(&self, ctx: &mut Context<'_, F>) -> Result<(), Error> {
        for _ in 0..self.max_available_rows / STEP_SIZE {
            ctx.region.assign_fixed(
                || "bit table: block sel",
                self.config.block_sel,
                ctx.offset + 1,
                || Ok(F::one()),
            )?;

            for i in [2, 3, 4, 5, 7, 8, 9, 10] {
                ctx.region.assign_fixed(
                    || "bit table: lookup sel",
                    self.config.lookup_sel,
                    ctx.offset + i,
                    || Ok(F::one()),
                )?;
            }

            for i in [1, 6] {
                ctx.region.assign_fixed(
                    || "bit table: u32 sel",
                    self.config.u32_sel,
                    ctx.offset + i,
                    || Ok(F::one()),
                )?;
            }

            ctx.step(STEP_SIZE);
        }

        Ok(())
    }

    fn assign_op(&self, ctx: &mut Context<'_, F>, op: BitTableOp) -> Result<(), Error> {
        for i in 0..STEP_SIZE {
            ctx.region.assign_advice(
                || "bit table op",
                self.config.op,
                ctx.offset + i,
                || Ok(F::from(op.index() as u64)),
            )?;
        }

        Ok(())
    }

    fn assign_u64_popcnt(
        &self,
        ctx: &mut Context<'_, F>,
        col: Column<Advice>,
        value: u64,
    ) -> Result<(), Error> {
        let count_ones = value.to_le_bytes();

        let low_u32 = count_ones[0..4]
            .iter()
            .map(|v| v.count_ones())
            .fold(0, |acc, v| acc + v);

        let high_u32 = count_ones[4..8]
            .iter()
            .map(|v| v.count_ones())
            .fold(0, |acc, v| acc + v);

        ctx.region.assign_advice(
            || "bit table: assign u64",
            col,
            ctx.offset,
            || Ok(F::from(value.count_ones() as u64)),
        )?;

        macro_rules! assign_u32 {
            ($v: expr, $bytes: expr, $offset: expr) => {{
                ctx.region.assign_advice(
                    || "bit table: assign u32",
                    col,
                    ctx.offset + $offset,
                    || Ok(F::from($v as u64)),
                )?;

                for (index, count_one) in $bytes.iter().enumerate() {
                    ctx.region.assign_advice(
                        || "bit table: assign u8",
                        col,
                        ctx.offset + 1 + index + $offset,
                        || Ok(F::from(count_one.count_ones() as u64)),
                    )?;
                }
            }};
        }

        assign_u32!(low_u32, count_ones[0..4], 1);
        assign_u32!(high_u32, count_ones[4..8], 6);

        Ok(())
    }

    fn assign_u64_le(
        &self,
        ctx: &mut Context<'_, F>,
        col: Column<Advice>,
        value: u64,
    ) -> Result<(), Error> {
        let low_u32 = value as u32;
        let high_u32 = (value >> 32) as u32;

        ctx.region.assign_advice(
            || "bit table: assign u64",
            col,
            ctx.offset,
            || Ok(F::from(value)),
        )?;

        macro_rules! assign_u32 {
            ($v: expr, $offset: expr) => {{
                let bytes = $v.to_le_bytes();

                ctx.region.assign_advice(
                    || "bit table: assign u32",
                    col,
                    ctx.offset + $offset,
                    || Ok(F::from($v as u64)),
                )?;

                for (index, byte) in bytes.into_iter().enumerate() {
                    ctx.region.assign_advice(
                        || "bit table: assign u8",
                        col,
                        ctx.offset + 1 + index + $offset,
                        || Ok(F::from(byte as u64)),
                    )?;
                }
            }};
        }

        assign_u32!(low_u32, 1);
        assign_u32!(high_u32, 6);

        Ok(())
    }

    fn assign_entries(
        &self,
        ctx: &mut Context<'_, F>,
        entries: Vec<BitTableAssign>,
    ) -> Result<(), Error> {
        assert!(entries.len() <= self.max_available_rows / STEP_SIZE);

        for entry in entries {
            self.assign_op(ctx, entry.op)?;
            self.assign_u64_le(ctx, self.config.left, entry.left)?;
            self.assign_u64_le(ctx, self.config.right, entry.right)?;
            if entry.op == BitTableOp::Popcnt {
                self.assign_u64_popcnt(ctx, self.config.result, entry.result)?;
            } else {
                self.assign_u64_le(ctx, self.config.result, entry.result)?;
            }

            ctx.step(STEP_SIZE);
        }

        Ok(())
    }

    pub(crate) fn assign(
        &self,
        ctx: &mut Context<'_, F>,
        event_table: &EventTableWithMemoryInfo,
    ) -> Result<(), Error> {
        self.init(ctx)?;

        ctx.reset();

        self.assign_entries(ctx, filter_bit_table_entries(event_table))?;

        Ok(())
    }
}
