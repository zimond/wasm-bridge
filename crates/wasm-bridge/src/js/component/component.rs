use anyhow::{bail, Context};
use js_sys::{Function, Object, Reflect, Uint8Array, WebAssembly};
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;

use crate::{
    direct::{LazyModuleMemory, ModuleMemory},
    helpers::{map_js_error, static_str_to_js},
    DropHandles, Engine, Result,
};

use super::*;

#[derive(Clone)]
pub struct Component {
    main_module: WebAssembly::Module,
    wasi_module: Option<WebAssembly::Module>,
}

impl Component {
    #[deprecated(
        since = "0.4.0",
        note = "Compiling a component synchronously can panic on the web, please use `new_safe` instead."
    )]
    pub fn new(_engine: &Engine, bytes: impl AsRef<[u8]>) -> Result<Self> {
        let files = ComponentLoader::generate_files(bytes.as_ref())?;

        let main_module = WebAssembly::Module::new(&bytes_to_js_value(&files.main_core))
            .map_err(map_js_error("Synchronously compile main core module"))?;

        let wasi_module = if let Some(wasi_core) = files.wasi_core {
            Some(
                WebAssembly::Module::new(&bytes_to_js_value(&wasi_core))
                    .map_err(map_js_error("Synchronously compile wasi core module"))?,
            )
        } else {
            None
        };

        Ok(Self {
            main_module,
            wasi_module,
        })
    }

    pub async fn new_safe(_engine: &Engine, bytes: impl AsRef<[u8]>) -> Result<Self> {
        let files = ComponentLoader::generate_files(bytes.as_ref())?;

        let promise = WebAssembly::compile(&bytes_to_js_value(&files.main_core));
        let module = JsFuture::from(promise)
            .await
            .map_err(map_js_error("Asynchronously compile main core module"))?;
        let main_module = module.into();

        let wasi_module = if let Some(wasi_core) = files.wasi_core {
            let promise = WebAssembly::compile(&bytes_to_js_value(&wasi_core));
            let module = JsFuture::from(promise)
                .await
                .map_err(map_js_error("Asynchronously compile wasi core module"))?;
            Some(module.into())
        } else {
            None
        };

        Ok(Self {
            main_module,
            wasi_module,
        })
    }

    pub(crate) fn is_wasi(&self) -> bool {
        self.wasi_module.is_some()
    }

    pub(crate) fn instantiate(
        &self,
        imports: &Object,
        drop_handles: DropHandles,
        lazy_memory: &LazyModuleMemory,
    ) -> Result<Instance> {
        let main_instance = WebAssembly::Instance::new(&self.main_module, imports)
            .map_err(map_js_error("Synchronously instantiate main module"))?;

        let memory = Self::create_module_memory(&main_instance, "cabi_realloc")?;
        lazy_memory.set(memory.clone());

        Instance::new(main_instance, drop_handles, &memory)
    }

    pub(crate) async fn instantiate_async(
        &self,
        imports: &Object,
        drop_handles: DropHandles,
        lazy_memory: &LazyModuleMemory,
    ) -> Result<Instance> {
        let promise = WebAssembly::instantiate_module(&self.main_module, imports);
        let instance = JsFuture::from(promise)
            .await
            .map_err(map_js_error("Asynchronously instantiate main module"))?;
        let main_instance = instance.into();

        let memory = Self::create_module_memory(&main_instance, "cabi_realloc")?;
        lazy_memory.set(memory.clone());

        Instance::new(main_instance, drop_handles, &memory)
    }

    pub(crate) fn instantiate_wasi(
        &self,
        imports: &Object,
        drop_handles: DropHandles,
        lazy_memory: &LazyModuleMemory,
        (wasi_imports, dyn_fns, lazy_wasi_memory): WasiInfo,
    ) -> Result<Instance> {
        let main_instance = WebAssembly::Instance::new(&self.main_module, imports)
            .map_err(map_js_error("Synchronously instantiate main module"))?;

        let main_memory = Self::prepare_wasi_imports(&main_instance, &wasi_imports)?;
        lazy_memory.set(main_memory.clone());

        let wasi_instance = WebAssembly::Instance::new(
            self.wasi_module.as_ref().context("Get wasi module")?,
            &wasi_imports,
        )
        .map_err(map_js_error("Synchronously instantiate wasi module"))?;

        let wasi_memory = Self::create_module_memory_from_js_memory(
            &wasi_instance,
            "cabi_import_realloc",
            main_memory.memory.memory.clone(),
        )?;
        lazy_wasi_memory.set(wasi_memory);

        Self::link_wasi_exports(&wasi_instance, &dyn_fns)?;

        Instance::new(main_instance, drop_handles, &main_memory)
    }

    pub(crate) async fn instantiate_wasi_async(
        &self,
        imports: &Object,
        drop_handles: DropHandles,
        lazy_memory: &LazyModuleMemory,
        (wasi_imports, dyn_fns, lazy_wasi_memory): WasiInfo,
    ) -> Result<Instance> {
        let promise = WebAssembly::instantiate_module(&self.main_module, imports);
        let instance = JsFuture::from(promise)
            .await
            .map_err(map_js_error("Asynchronously instantiate main module"))?;
        let main_instance: WebAssembly::Instance = instance.into();

        let main_memory = Self::prepare_wasi_imports(&main_instance, &wasi_imports)?;
        lazy_memory.set(main_memory.clone());

        let promise = WebAssembly::instantiate_module(
            self.wasi_module.as_ref().context("Get wasi module")?,
            &wasi_imports,
        );
        let instance = JsFuture::from(promise)
            .await
            .map_err(map_js_error("Asynchronously instantiate wasi module"))?;
        let wasi_instance: WebAssembly::Instance = instance.into();

        let wasi_memory = Self::create_module_memory_from_js_memory(
            &wasi_instance,
            "cabi_import_realloc",
            main_memory.memory.memory.clone(),
        )?;
        lazy_wasi_memory.set(wasi_memory);

        Self::link_wasi_exports(&wasi_instance, &dyn_fns)?;

        Instance::new(main_instance, drop_handles, &main_memory)
    }

    fn prepare_wasi_imports(
        main_instance: &WebAssembly::Instance,
        wasi_imports: &Object,
    ) -> Result<ModuleMemory> {
        let main_module_memory = Self::create_module_memory(main_instance, "cabi_realloc")?;

        // cabi_realloc in __main_module__
        let main_module_obj = Object::new();
        Reflect::set(
            &main_module_obj,
            static_str_to_js("cabi_realloc"),
            &main_module_memory.realloc,
        )
        .expect("main module is an object");

        Reflect::set(
            wasi_imports,
            static_str_to_js("__main_module__"),
            &main_module_obj,
        )
        .expect("wasi imports is an object");

        // memory in env
        let env_obj = Object::new();
        Reflect::set(
            &env_obj,
            static_str_to_js("memory"),
            &main_module_memory.memory.memory,
        )
        .expect("env is an object");

        Reflect::set(wasi_imports, static_str_to_js("env"), &env_obj)
            .expect("wasi imports is an object");

        Ok(main_module_memory)
    }

    fn link_wasi_exports(wasi_instance: &WebAssembly::Instance, dyn_fns: &DynFns) -> Result<()> {
        let wasi_exports = wasi_instance.exports();

        for (name, dyn_fn) in dyn_fns {
            let exported_fn = Reflect::get(&wasi_exports, &(*name).into())
                .map_err(map_js_error("wasi exports get fn"))?;

            // If the function is missing, we ignore it, only used imports are present
            if exported_fn.is_function() {
                Reflect::set_u32(dyn_fn, 0, &exported_fn).expect("dyn_fn is an array");
            }
        }

        Ok(())
    }

    fn create_module_memory(
        instance: &WebAssembly::Instance,
        realloc_name: &'static str,
    ) -> Result<ModuleMemory> {
        let exports = instance.exports();

        let memory = Reflect::get(&exports, static_str_to_js("memory"))
            .map_err(map_js_error("get memory export"))?;
        if !memory.is_object() {
            bail!("Instance's memory is not an object, it's {memory:?} instead.");
        }
        let memory: WebAssembly::Memory = memory.into();

        Self::create_module_memory_from_js_memory(instance, realloc_name, memory)
    }

    fn create_module_memory_from_js_memory(
        instance: &WebAssembly::Instance,
        realloc_name: &'static str,
        existing_memory: WebAssembly::Memory,
    ) -> Result<ModuleMemory> {
        let exports = instance.exports();

        let realloc = Reflect::get(&exports, static_str_to_js(realloc_name))
            .map_err(map_js_error("get realloc export"))?;
        if !realloc.is_function() {
            bail!(
                "Instance's realloc '{realloc_name}' is not a function, it's {realloc:?} instead."
            );
        }
        let realloc: Function = realloc.into();

        Ok(ModuleMemory::new(
            crate::Memory::new(existing_memory),
            realloc,
        ))
    }
}

fn bytes_to_js_value(bytes: &[u8]) -> JsValue {
    Uint8Array::from(bytes).into()
}
