use wasmtime::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load the wasm module.
    let serialized_module_file =
        "../implementation_module/target/wasm32-unknown-unknown/debug/implementation_module.wasm";

    let mut config = wasmtime::Config::default();
    config.consume_fuel(true);
    // config.epoch_interruption(true);

    #[derive(Debug)]
    struct MyStorage {
        pub a: i32,
    };

    // An engine stores and configures global compilation settings like
    // optimization level, enabled wasm features, etc.

    let storage = MyStorage { a: 0 };
    let engine = Engine::new(&config)?;
    let mut linker = Linker::<MyStorage>::new(&engine);

    fn foo() {
        println!("Foo called from wasm");
    }
    linker.func_wrap("env", "foo", || foo())?;

    linker.func_wrap(
        "env",
        "log_record",
        |mut caller: Caller<'_, MyStorage>, ptr: i32, len: i32| {
            // println!("Pointer: caller: {caller:?}");
            println!("Pointer: {ptr:?}, len: {len}");

            let mem = match caller.get_export("memory") {
                Some(Extern::Memory(mem)) => mem,
                _ => {
                    println!("failed to find host memory");
                    return;
                }
            };
            let data = mem
                .data(&caller)
                .get(ptr as u32 as usize..)
                .and_then(|arr| arr.get(..len as u32 as usize));
            println!("data: {data:?}");
            let string = match data {
                Some(data) => match std::str::from_utf8(data) {
                    Ok(s) => s,
                    Err(_) => "buhu",
                },
                None => "out of bounds",
            };
            println!("string: {string:?}");

            caller.data_mut().a = 3;
        },
    );

    // We start off by creating a `Module` which represents a compiled form
    // of our input wasm module. In this case it'll be JIT-compiled after
    // we parse the text format.
    let module = Module::from_file(&engine, serialized_module_file)?;

    // A `Store` is what will own instances, functions, globals, etc. All wasm
    // items are stored within a `Store`, and it's what we'll always be using to
    // interact with the wasm world. Custom data can be stored in stores but for
    // now we just use `()`.
    let mut store = Store::new(&engine, storage);
    store.add_fuel(10000000)?;
    // store.epoch_deadline_trap();
    // store.set_epoch_deadline(1);
    // engine.increment_epoch();

    // With a compiled `Module` we can then instantiate it, creating
    // an `Instance` which we can actually poke at functions on.
    // let instance = Instance::new(&mut store, &module, &[])?;
    let instance = linker.instantiate(&mut store, &module)?;

    // The `Instance` gives us access to various exported functions and items,
    // which we access here to pull out our `answer` exported function and
    // run it.
    let sum = instance
        .get_func(&mut store, "sum")
        .expect("`answer` was not an exported function");

    // There's a few ways we can call the `answer` `Func` value. The easiest
    // is to statically assert its signature with `typed` (in this case
    // asserting it takes no arguments and returns one i32) and then call it.
    let sum_fun = sum.typed::<(i32, i32), i32, _>(&store)?;

    // And finally we can call our function! Note that the error propagation
    // with `?` is done to handle the case where the wasm function traps.
    for i in 0..1000000i32 {
        let result = match sum_fun.call(&mut store, (i, i)) {
            Ok(v) => v,
            Err(z) => {
                println!("z: {z:?}");
                break;
            }
        };
        // println!("Answer: {:?}", result);
    }

    // Call the imported 'foo' symbol.
    let call_foo = instance
        .get_func(&mut store, "call_foo")
        .expect("`call_foo` was not an exported function");
    call_foo.call(&mut store, &[], &mut [])?;

    // Try the logger, this also does guest -> host
    // Also tests guest -> Storage
    {
        // log test
        let log_setup = instance
            .get_func(&mut store, "log_setup")
            .expect("`call_foo` was not an exported function");
        log_setup.call(&mut store, &[], &mut [])?;
        let log_test = instance
            .get_func(&mut store, "log_test")
            .expect("`log_test` was not an exported function");
        log_test.call(&mut store, &[], &mut [])?;
    }

    // Test input, this does host -> guest
    {
        let prepare_input = instance
            .get_func(&mut store, "prepare_input")
            .expect("should have this");
        println!("prepare_input: {prepare_input:?}");
        let prepare_input_typed = prepare_input.typed::<u32, i32, _>(&store)?;

        let content = [0u8, 1, 2, 3, 4, 5, 6, 7, 8];
        let len = content.len() as u32;
        let res_ptr = prepare_input_typed.call(&mut store, len)?;
        // Now we have the pointer... we can write into that, somehow.

        let mem = instance.get_memory(&mut store, "memory").unwrap();
        let (bytes, storage) = mem.data_and_store_mut(&mut store);
        println!("Bytes length: {}", bytes.len());
        // Now, we can write...
        bytes[res_ptr as usize..(res_ptr as u32 + len) as usize].clone_from_slice(&content);

        let use_input = instance
            .get_func(&mut store, "use_input")
            .expect("use-input not found");
        let use_input_typed = use_input.typed::<u32, (), _>(&store)?;

        use_input_typed.call(&mut store, len)?;
    }

    // test sin
    {
        let test_sin = instance
            .get_func(&mut store, "test_sin")
            .expect("test_sin missing");
        let test_sin_typed = test_sin.typed::<f32, f32, _>(&store)?;
        assert_eq!(test_sin_typed.call(&mut store, 1.0)?, 1.0f32.sin());
        assert_eq!(test_sin_typed.call(&mut store, 2.0)?, 2.0f32.sin());
    }

    println!("store.T: {:?}", store.data());

    Ok(())
}
