use halo2_proofs::arithmetic::best_multiexp_gpu_cond;
use halo2_proofs::arithmetic::CurveAffine;
use halo2_proofs::poly::commitment::Params;
use specs::ImageTable;

use crate::circuits::image_table::EncodeImageTableValues;

pub trait ImageCheckSum<Output> {
    fn checksum(&self) -> Output;
}

pub(crate) struct ImageTableWithParams<'a, 'b, C: CurveAffine> {
    pub(crate) table: &'a ImageTable,
    pub(crate) params: &'b Params<C>,
}

impl<'a, 'b, C: CurveAffine> ImageCheckSum<Vec<C>> for ImageTableWithParams<'a, 'b, C> {
    fn checksum(&self) -> Vec<C> {
        let cells = self.table.encode_image_table_values().plain();

        let c = best_multiexp_gpu_cond(&cells[..], &self.params.get_g_lagrange()[0..cells.len()]);
        vec![c.into()]
    }
}
