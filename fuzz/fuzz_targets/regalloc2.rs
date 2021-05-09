#![no_main]
use libfuzzer_sys::fuzz_target;

use minira::{self, test_framework as ir};
use regalloc;

// This is the number of test cases the fuzzing framework has given to us so
// far.  More then half of these get rejected as having unreachable blocks, or
// critical edges, or live values into the start node, or for whatever reason
// they are invalid.  Hence ..
static mut COUNTER_GEN: usize = 0;

// .. this is used to count the number of test cases which actually made it
// through the allocator.  This number gives a better measure of the extent of
// test coverage.
static mut COUNTER_OK: usize = 0;

fuzz_target!(|func: ir::Func| {
    let _ = env_logger::try_init();
    let n_gen = unsafe {
        COUNTER_GEN += 1;
        COUNTER_GEN
    };
    let n_ok = unsafe { COUNTER_OK };
    println!(
        "==== BEGIN fuzz_regalloc2.rs: #gen'd {:?} #ok {} ========================",
        n_gen, n_ok
    );

    if false {
        println!("BEGIN INPUT:");
        let mut rendered = String::new();
        func.render("==== fuzz_regalloc2.rs: input:", &mut rendered)
            .unwrap();
        println!("{}", rendered);
        println!("END INPUT:");
    }

    let mut func = func;

    let num_regs = minira::fuzzing::NUM_REAL_REGS_PER_RC as usize;
    let reg_universe = ir::make_universe(num_regs, num_regs);

    let func_backup = func.clone();

    let opts = regalloc::Options {
        run_checker: true,

        algorithm: regalloc::Algorithm::Regalloc2(regalloc::Regalloc2Options {
            num_int_preferred: 4,
            num_float_preferred: 4,
        }),
    };

    let sri = func.get_stackmap_request();
    let ra_result =
        regalloc::allocate_registers_with_opts(&mut func, &reg_universe, sri.as_ref(), opts);

    match ra_result {
        Ok(result) => {
            func.update_from_alloc(result);
            unsafe {
                COUNTER_OK += 1;
            }
            return;
        }
        Err(err) => {
            let mut stop = false;
            if let regalloc::RegAllocError::RegChecker(_) = &err {
                stop = true;
                println!("==== fuzz_regalloc2.rs: checker error: {:?}", err);
            }
            if stop {
                let mut rendered = String::new();
                func_backup
                    .render("==== fuzz_regalloc2.rs: failing input:", &mut rendered)
                    .unwrap();
                println!("{}", rendered);
            }
            println!("==== fuzz_regalloc2.rs: failure reason: {}", err);
            if stop {
                println!("==== fuzz_regalloc2.rs:");
                println!(
                    "==== fuzz_regalloc2.rs: to repro, use flags '-f {} -i {}'",
                    num_regs, num_regs
                );
                println!("==== fuzz_regalloc2.rs:");
                panic!("==== fuzz_regalloc2.rs: STOPPING.  Bye! ====");
            }
        }
    };
});
