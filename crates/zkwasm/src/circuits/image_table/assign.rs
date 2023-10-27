use halo2_proofs::arithmetic::FieldExt;
use halo2_proofs::circuit::Cell;
use halo2_proofs::circuit::Layouter;
use halo2_proofs::plonk::Advice;
use halo2_proofs::plonk::Column;
use halo2_proofs::plonk::Error;
use specs::InitializationState;

use super::ImageTableChip;
use super::ImageTableLayouter;
use crate::circuits::utils::Context;

impl<F: FieldExt> ImageTableChip<F> {
    pub fn assign(
        self,
        layouter: &mut impl Layouter<F>,
        image_table: ImageTableLayouter<F>,
        permutation_cells: ImageTableLayouter<Cell>,
    ) -> Result<(), Error> {
        fn assign_and_constrain_initialization_state<F: FieldExt>(
            ctx: &mut Context<F>,
            col: Column<Advice>,
            initialization_state: &InitializationState<F>,
            permutation_cells: &InitializationState<Cell>,
        ) -> Result<(), Error> {
            macro_rules! assign_and_constraint {
                ($field:ident) => {
                    let cell = ctx
                        .region
                        .assign_advice(
                            || "image table",
                            col,
                            ctx.offset,
                            || Ok(initialization_state.$field),
                        )?
                        .cell();
                    println!("assign imag iid:{:?}", initialization_state.$field);
                    ctx.region.constrain_equal(cell, permutation_cells.$field)?;
                    ctx.next();
                };
            }

            assign_and_constraint!(eid);
            assign_and_constraint!(fid);
            assign_and_constraint!(iid);
            assign_and_constraint!(frame_id);
            assign_and_constraint!(sp);
            assign_and_constraint!(initial_memory_pages);
            assign_and_constraint!(maximal_memory_pages);

            assign_and_constraint!(input_index);
            assign_and_constraint!(context_input_index);
            assign_and_constraint!(context_output_index);
            assign_and_constraint!(external_host_call_index);

            assign_and_constraint!(jops);

            Ok(())
        }

        layouter.assign_region(
            || "image table",
            |region| {
                let mut ctx = Context::new(region);

                macro_rules! assign_one_line {
                    ($v: expr) => {{
                        let cell = ctx
                            .region
                            .assign_advice(
                                || "image table",
                                self.config.col,
                                ctx.offset,
                                || Ok($v),
                            )?
                            .cell();

                        ctx.next();

                        cell
                    }};
                }

                assign_and_constrain_initialization_state(
                    &mut ctx,
                    self.config.col,
                    &image_table.initialization_state,
                    &permutation_cells.initialization_state,
                )?;

                for (static_frame_entry, cell_in_frame_table) in image_table
                    .static_frame_entries
                    .iter()
                    .zip(permutation_cells.static_frame_entries.iter())
                {
                    // Enable cell
                    let cell = assign_one_line!(static_frame_entry.0);
                    ctx.region.constrain_equal(cell, cell_in_frame_table.0)?;

                    let cell = assign_one_line!(static_frame_entry.1);
                    ctx.region.constrain_equal(cell, cell_in_frame_table.1)?;
                }

                for value in image_table.lookup_entries.as_ref().unwrap() {
                    assign_one_line!(*value);
                }

                Ok(())
            },
        )
    }
}
