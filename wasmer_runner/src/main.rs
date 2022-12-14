use wasmer::{
    imports, EngineBuilder, Function, FunctionEnv, FunctionEnvMut, Instance, Memory, MemoryType,
    Module, Store, TypedFunction, Value, WasmPtr,
};
use wasmer_compiler_cranelift::Cranelift;

use std::sync::Arc;
use wasmer::wasmparser::Operator;

use wasmer::CompilerConfig;

use wasmer_middlewares::{
    metering::{get_remaining_points, set_remaining_points},
    Metering,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // cost function from https://github.com/wasmerio/wasmer/blob/v3.0.0-rc.2/examples/metering.rs

    let cost_function = |operator: &Operator| -> u64 {
        match operator {
            Operator::LocalGet { .. } | Operator::I32Const { .. } => 1,
            Operator::I32Add { .. } => 2,
            _ => 0,
        }
    };

    let metering = Arc::new(Metering::new(10000000, cost_function));
    let mut compiler_config = Cranelift::default();
    compiler_config.push_middleware(metering);
    let mut store = Store::new(EngineBuilder::new(compiler_config));

    // Function to be provided from this side.
    struct MyEnv;
    let env = FunctionEnv::new(&mut store, MyEnv {});
    fn foo(_env: FunctionEnvMut<MyEnv>) {
        println!("Foo called");
    }
    let foo_typed = Function::new_typed_with_env(&mut store, &env, foo);

    // https://github.com/wasmerio/wasmer/issues/3274
    // https://github.com/wasmerio/wasmer/issues/2255

    // Put memory in an arc, we initialise it with a dummy here, then later replace it with the
    // program's real memory.
    let my_memory = std::sync::Arc::new(std::sync::Mutex::new(
        Memory::new(&mut store, MemoryType::new(1, None, false)).unwrap(),
    ));
    struct EnvWithMemory {
        memory: std::sync::Arc<std::sync::Mutex<Memory>>,
    }

    let env_with_mem = FunctionEnv::new(
        &mut store,
        EnvWithMemory {
            memory: my_memory.clone(),
        },
    );

    fn log_record(env: FunctionEnvMut<EnvWithMemory>, ptr: WasmPtr<u8>, len: u32) {
        println!("Pointer: {ptr:?}, len: {len}");
        let locked_memory = env.data().memory.lock().expect("klsdjflsd");
        let mem = locked_memory.view(&env);
        // let outbuf: WasmPtr<u8> = unsafe{ std::mem::transmute(ptr) };
        println!("ptr: {ptr:?}");
        let mut read_back = Vec::<u8>::new();
        read_back.resize(len as usize, 0);
        // let mut v = [0u8; len];
        let mem_slice = ptr.slice(&mem, len).unwrap();
        mem_slice.read_slice(&mut read_back[..]);
        println!("read_back: {read_back:?}");
        let s = std::str::from_utf8(&read_back).unwrap();
        println!("outbuf: {s:?}");
    }
    let log_record_typed = Function::new_typed_with_env(&mut store, &env_with_mem, log_record);

    let import_object = imports! {
        "env" => {
            "foo" => foo_typed,
            // "print_string" => print_string_typed,
            "log_record" => log_record_typed,
            // "get_memory_typed" => get_memory_typed,
        }
    };

    // Load the wasm module.
    let serialized_module_file =
        "../implementation_module/target/wasm32-unknown-unknown/debug/implementation_module.wasm";
    let module = Module::from_file(&store, serialized_module_file)?;
    println!("Module: {module:?}");
    for export_ in module.exports() {
        println!("{:?}", export_.ty());
    }

    println!("Instantiating module...");
    let instance = Instance::new(&mut store, &module, &import_object)?;

    // Now that we have the instance, swap the pointer from the my_memory.
    let memory = instance.exports.get_memory("memory")?.clone();
    // *std::sync::Arc::<std::sync::Mutex::<wasmer::Memory>>::get_mut(&mut my_memory).get_mut().unwrap() = std::sync::Mutex::new(memory);
    *my_memory.lock().unwrap() = memory;

    println!("points: {:?}", get_remaining_points(&mut store, &instance));

    set_remaining_points(&mut store, &instance, 10000000);

    // Test sum.
    {
        // Get the function.
        let sum = instance.exports.get_function("sum")?;

        println!("Calling `sum` function...");

        let args = [Value::I32(1), Value::I32(5)];
        let result = sum.call(&mut store, &args)?;
        println!("points: {:?}", get_remaining_points(&mut store, &instance));

        println!("Results: {:?}", result);
        assert_eq!(result.to_vec(), vec![Value::I32(1 + 5)]);

        // Call it as a typed function.
        let sum_typed: TypedFunction<(i32, i32), i32> = sum.typed(&mut store)?;

        println!("Calling `sum` function (natively)...");
        let result = sum_typed.call(&mut store, 1, 5)?;

        println!("Results: {:?}", result);
        assert_eq!(result, 6);
        println!("points: {:?}", get_remaining_points(&mut store, &instance));
    }

    // test foo
    {
        let call_foo = instance.exports.get_function("call_foo")?;
        let foo_typed: TypedFunction<(), ()> = call_foo.typed(&mut store)?;
        let _res = foo_typed.call(&mut store)?;
    }

    // test alloc
    {
        let sum_with_alloc = instance.exports.get_function("sum_with_alloc")?;
        let sum_with_alloc_typed: TypedFunction<u64, u64> = sum_with_alloc.typed(&mut store)?;
        let result = sum_with_alloc_typed.call(&mut store, 100)?;
        assert_eq!(result, 197);
    }

    // test opaque state
    {
        let set_state = instance.exports.get_function("set_state")?;
        let get_state = instance.exports.get_function("get_state")?;
        let set_state_typed: TypedFunction<u32, ()> = set_state.typed(&mut store)?;
        let get_state_typed: TypedFunction<(), u32> = get_state.typed(&mut store)?;
        let _result = set_state_typed.call(&mut store, 100)?;
        assert_eq!(get_state_typed.call(&mut store)?, 100u32);
        let _result = set_state_typed.call(&mut store, 101)?;
        assert_eq!(get_state_typed.call(&mut store)?, 101u32);
    }

    // Try the handler
    {
        let setup_handler = instance.exports.get_function("setup_handler")?;
        let setup_handler_typed: TypedFunction<(), ()> = setup_handler.typed(&mut store)?;
        let _res = setup_handler_typed.call(&mut store)?;

        let call_handler = instance.exports.get_function("call_handler")?;
        let call_handler_typed: TypedFunction<(), ()> = call_handler.typed(&mut store)?;
        let _res = call_handler_typed.call(&mut store)?;
    }

    // Try the logger, this also does guest -> host
    {
        let log_setup = instance.exports.get_function("log_setup")?;
        let log_setup_typed: TypedFunction<(), ()> = log_setup.typed(&mut store)?;
        let _res = log_setup_typed.call(&mut store)?;
        let log_test = instance.exports.get_function("log_test")?;
        let log_test_typed: TypedFunction<(), ()> = log_test.typed(&mut store)?;
        let _res = log_test_typed.call(&mut store)?;
    }

    // Test input, this does host -> guest
    {
        let prepare_input = instance.exports.get_function("prepare_input")?;
        let prepare_input_typed: TypedFunction<u32, WasmPtr<u8>> =
            prepare_input.typed(&mut store)?;

        let content = [0u8, 1, 2, 3, 4, 5, 6, 7, 8];
        let len = content.len() as u32;
        let res_ptr = prepare_input_typed.call(&mut store, len)?;
        // Now we have the pointer... we can write into that, somehow.
        let memory = instance.exports.get_memory("memory")?.clone();
        let view = memory.view(&store);
        let mem_slice = res_ptr.slice(&view, len).unwrap();
        mem_slice.write_slice(&content);

        let use_input = instance.exports.get_function("use_input")?;
        let use_input_typed: TypedFunction<u32, ()> = use_input.typed(&mut store)?;
        let _res = use_input_typed.call(&mut store, len)?;
    }

    // test sin
    {
        let test_sin = instance.exports.get_function("test_sin")?;
        let test_sin_typed: TypedFunction<f32, f32> = test_sin.typed(&mut store)?;
        assert_eq!(test_sin_typed.call(&mut store, 1.0)?, 1.0f32.sin());
        assert_eq!(test_sin_typed.call(&mut store, 2.0)?, 2.0f32.sin());
    }

    Ok(())
}
