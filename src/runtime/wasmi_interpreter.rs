use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::circuits::config::zkwasm_k;
use crate::runtime::memory_event_of_step;
use anyhow::Result;
use ark_std::end_timer;
use ark_std::start_timer;
use rayon::prelude::ParallelIterator;
use rayon::slice::ParallelSlice;
use specs::host_function::HostFunctionDesc;
use specs::jtable::StaticFrameEntry;
use specs::mtable::MTable;
use specs::CompilationTable;
use specs::ExecutionTable;
use specs::Tables;
use wasmi::Externals;
use wasmi::ImportResolver;
use wasmi::ModuleInstance;
use wasmi::RuntimeValue;

use super::CompiledImage;
use super::ExecutionResult;

pub struct WasmRuntimeIO {
    pub public_inputs_and_outputs: Rc<RefCell<Vec<u64>>>,
    pub outputs: Rc<RefCell<Vec<u64>>>,
}

impl WasmRuntimeIO {
    pub fn empty() -> Self {
        Self {
            public_inputs_and_outputs: Rc::new(RefCell::new(vec![])),
            outputs: Rc::new(RefCell::new(vec![])),
        }
    }
}

pub trait Execution<R> {
    fn run<E: Externals>(
        self,
        externals: &mut E,
        wasm_io: WasmRuntimeIO,
    ) -> Result<ExecutionResult<R>>;
}

impl Execution<RuntimeValue>
    for CompiledImage<wasmi::NotStartedModuleRef<'_>, wasmi::tracer::Tracer>
{
    fn run<E: Externals>(
        self,
        externals: &mut E,
        wasm_io: WasmRuntimeIO,
    ) -> Result<ExecutionResult<RuntimeValue>> {
        let timer = start_timer!(|| "invoke start");
        let instance = self
            .instance
            .run_start_tracer(externals, self.tracer.clone())
            .unwrap();
        end_timer!(timer);

        let timer = start_timer!(|| "invoke export");
        let result =
            instance.invoke_export_trace(&self.entry, &[], externals, self.tracer.clone())?;
        end_timer!(timer);

        let timer = start_timer!(|| "prepare table");
        let execution_tables = {
            let tracer = RefCell::into_inner(Rc::try_unwrap(self.tracer).unwrap());

            let timer = start_timer!(|| "prepare mtable");
            let mtable = {
                let groups = rayon::current_num_threads();
                let chunk_size = tracer.etable.entries().len().div_ceil(groups);

                let timer = start_timer!(|| "prepare mtable core");
                let mentries = tracer
                    .etable
                    .entries()
                    .par_chunks(chunk_size)
                    .map(|slot| {
                        slot.iter()
                            .flat_map(|eentry| memory_event_of_step(eentry, &mut 1))
                            .collect()
                    })
                    .collect::<Vec<Vec<_>>>()
                    .concat();
                end_timer!(timer);

                MTable::new(mentries, &self.tables.imtable)
            };
            end_timer!(timer);

            ExecutionTable {
                etable: tracer.etable,
                mtable,
                jtable: tracer.jtable,
            }
        };
        end_timer!(timer);

        Ok(ExecutionResult {
            tables: Tables {
                compilation_tables: self.tables.clone(),
                execution_tables,
            },
            result,
            public_inputs_and_outputs: wasm_io.public_inputs_and_outputs.borrow().clone(),
            outputs: wasm_io.public_inputs_and_outputs.borrow().clone(),
        })
    }
}

pub struct WasmiRuntime;

impl WasmiRuntime {
    pub fn new() -> Self {
        WasmiRuntime
    }

    pub fn compile<'a, I: ImportResolver>(
        &self,
        module: &'a wasmi::Module,
        imports: &I,
        host_plugin_lookup: &HashMap<usize, HostFunctionDesc>,
        entry: &str,
    ) -> Result<CompiledImage<wasmi::NotStartedModuleRef<'a>, wasmi::tracer::Tracer>> {
        let tracer = wasmi::tracer::Tracer::new(host_plugin_lookup.clone());
        let tracer = Rc::new(RefCell::new(tracer));

        let instance = ModuleInstance::new(&module, imports, Some(tracer.clone()))
            .expect("failed to instantiate wasm module");

        let fid_of_entry = {
            let idx_of_entry = instance.lookup_function_by_name(tracer.clone(), entry);

            if instance.has_start() {
                tracer
                    .clone()
                    .borrow_mut()
                    .static_jtable_entries
                    .push(StaticFrameEntry {
                        frame_id: 0,
                        next_frame_id: 0,
                        callee_fid: 0, // the fid of start function is always 0
                        fid: idx_of_entry,
                        iid: 0,
                    });
            }

            tracer
                .clone()
                .borrow_mut()
                .static_jtable_entries
                .push(StaticFrameEntry {
                    frame_id: 0,
                    next_frame_id: 0,
                    callee_fid: idx_of_entry,
                    fid: 0,
                    iid: 0,
                });

            if instance.has_start() {
                0
            } else {
                idx_of_entry
            }
        };

        let itable = tracer.borrow().itable.clone();
        let imtable = tracer.borrow().imtable.finalized(zkwasm_k());
        let elem_table = tracer.borrow().elem_table.clone();
        let configure_table = tracer.borrow().configure_table.clone();
        let static_jtable = tracer.borrow().static_jtable_entries.clone();

        Ok(CompiledImage {
            entry: entry.to_owned(),
            tables: CompilationTable {
                itable,
                imtable,
                elem_table,
                configure_table,
                static_jtable,
                fid_of_entry,
            },
            instance,
            tracer,
        })
    }
}
