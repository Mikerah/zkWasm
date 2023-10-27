use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use anyhow::Result;
use specs::host_function::HostFunctionDesc;
use specs::jtable::StaticFrameEntry;
use specs::ExecutionTable;
use specs::ImageTable;
use specs::InitializationState;
use specs::Tables;
use wasmi::Externals;
use wasmi::ImportResolver;
use wasmi::ModuleInstance;
use wasmi::RuntimeValue;
use wasmi::DEFAULT_VALUE_STACK_LIMIT;

use crate::circuits::config::zkwasm_k;

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
    fn dry_run<E: Externals>(self, externals: &mut E) -> Result<Option<R>>;

    fn run<E: Externals>(
        self,
        externals: &mut E,
        wasm_io: WasmRuntimeIO,
    ) -> Result<ExecutionResult<R>>;
}

impl Execution<RuntimeValue>
    for CompiledImage<wasmi::NotStartedModuleRef<'_>, wasmi::tracer::Tracer>
{
    fn dry_run<E: Externals>(self, externals: &mut E) -> Result<Option<RuntimeValue>> {
        let instance = self.instance.run_start(externals).unwrap();

        let result = instance.invoke_export(&self.entry, &[], externals)?;

        Ok(result)
    }

    fn run<E: Externals>(
        self,
        externals: &mut E,
        wasm_io: WasmRuntimeIO,
    ) -> Result<ExecutionResult<RuntimeValue>> {
        let instance = self
            .instance
            .run_start_tracer(externals, self.tracer.clone())
            .unwrap();

        let result =
            instance.invoke_export_trace(&self.entry, &[], externals, self.tracer.clone())?;

        let execution_table = {
            let tracer = self.tracer.borrow();

            let first_eentry = tracer.etable.entries().first().clone().unwrap();
            let last_eentry = tracer.etable.entries().last().clone().unwrap();

            ExecutionTable {
                etable: tracer.etable.clone(),
                jtable: tracer.jtable.clone(),
            }
        };

        let pre_image_table = self.tables.clone();
        let post_image_table = pre_image_table.update_image_table(&execution_table);

        Ok(ExecutionResult {
            tables: Tables {
                pre_image_table,
                post_image_table,
                execution_table,
            },
            result,
            public_inputs_and_outputs: wasm_io.public_inputs_and_outputs.borrow().clone(),
            outputs: wasm_io.outputs.borrow().clone(),
        })
    }
}

pub struct WasmiRuntime;

impl WasmiRuntime {
    pub fn new() -> Self {
        WasmiRuntime
    }

    pub fn compile<'a, I: ImportResolver>(
        module: &'a wasmi::Module,
        imports: &I,
        host_plugin_lookup: &HashMap<usize, HostFunctionDesc>,
        entry: &str,
        phantom_functions: &Vec<String>,
    ) -> Result<CompiledImage<wasmi::NotStartedModuleRef<'a>, wasmi::tracer::Tracer>> {
        let tracer = wasmi::tracer::Tracer::new(host_plugin_lookup.clone(), phantom_functions);
        let tracer = Rc::new(RefCell::new(tracer));

        let instance = ModuleInstance::new(&module, imports, Some(tracer.clone()))
            .expect("failed to instantiate wasm module");

        let fid_of_entry = {
            let idx_of_entry = instance.lookup_function_by_name(tracer.clone(), entry);

            tracer
                .clone()
                .borrow_mut()
                .static_jtable_entries
                .push(StaticFrameEntry {
                    enable: true,
                    frame_id: 0,
                    next_frame_id: 0,
                    callee_fid: idx_of_entry,
                    fid: 0,
                    iid: 0,
                });

            if instance.has_start() {
                tracer
                    .clone()
                    .borrow_mut()
                    .static_jtable_entries
                    .push(StaticFrameEntry {
                        enable: true,
                        frame_id: 0,
                        next_frame_id: 0,
                        callee_fid: 0, // the fid of start function is always 0
                        fid: idx_of_entry,
                        iid: 0,
                    });
            }

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
        let initialization_state = InitializationState {
            eid: 1,
            fid: fid_of_entry,
            iid: 0,
            frame_id: 0,
            sp: DEFAULT_VALUE_STACK_LIMIT as u32 - 1,
            initial_memory_pages: configure_table.init_memory_pages,
            maximal_memory_pages: configure_table.maximal_memory_pages,
            input_index: 1,
            context_input_index: 1,
            context_output_index: 1,
            external_host_call_index: 1,
            jops: 0,
        };

        Ok(CompiledImage {
            entry: entry.to_owned(),
            tables: ImageTable {
                itable,
                imtable,
                elem_table,
                static_jtable,
                initialization_state,
            },
            instance,
            tracer,
        })
    }
}
