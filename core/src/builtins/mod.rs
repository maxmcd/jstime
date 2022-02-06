use rand::prelude::*;
use std::convert::TryFrom;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
lazy_static! {
    pub(crate) static ref EXTERNAL_REFERENCES: v8::ExternalReferences =
        v8::ExternalReferences::new(&[
            v8::ExternalReference {
                function: v8::MapFnTo::map_fn_to(printer),
            },
            v8::ExternalReference {
                function: v8::MapFnTo::map_fn_to(performance_now),
            },
            v8::ExternalReference {
                function: v8::MapFnTo::map_fn_to(queue_microtask),
            },
            v8::ExternalReference {
                function: v8::MapFnTo::map_fn_to(set_timeout),
            },
            v8::ExternalReference {
                function: v8::MapFnTo::map_fn_to(random_float),
            },
        ]);
}

pub(crate) struct Builtins {}

impl Builtins {
    pub(crate) fn create(scope: &mut v8::HandleScope) {
        let bindings = v8::Object::new(scope);

        macro_rules! binding {
            ($name:expr, $fn:ident) => {
                let name = v8::String::new(scope, $name).unwrap();
                let value = v8::Function::new(scope, $fn).unwrap();
                bindings.set(scope, name.into(), value.into());
            };
        }

        binding!("printer", printer);
        binding!("perfNow", performance_now);
        binding!("queueMicrotask", queue_microtask);
        binding!("randomFloat", random_float);
        binding!("setTimeout", set_timeout);

        macro_rules! builtin {
            ($name:expr) => {
                let source = include_str!($name);
                let val = match crate::script::run(scope, source, $name) {
                    Ok(v) => v,
                    Err(_) => unreachable!(),
                };
                let func = v8::Local::<v8::Function>::try_from(val).unwrap();
                let recv = v8::undefined(scope).into();
                let args = [bindings.into()];
                func.call(scope, recv, &args).unwrap();
            };
        }

        builtin!("./console.js");
        builtin!("./crypto.js");
        builtin!("./timers.js");
        builtin!("./performance.js");
        builtin!("./encoders.js");
        builtin!("./queue_microtask.js");
    }
    pub(crate) fn init(scope: &mut v8::HandleScope) {
        scope.set_slot(TimerQueue::new());
        scope.set_slot(Instant::now() as TimeOrigin);
    }
}

fn random_float(
    scope: &mut v8::HandleScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let mut rng = rand::thread_rng();
    rv.set(v8::Number::new(scope, rng.gen::<f64>()).into());
}

type TimeOrigin = Instant;

fn performance_now(
    scope: &mut v8::HandleScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let time_origin = scope.get_slot::<TimeOrigin>().unwrap();
    let dur = time_origin.elapsed();
    rv.set(v8::Number::new(scope, dur.as_nanos() as f64 / 1e6).into());
}

fn printer(scope: &mut v8::HandleScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let arg_len = args.length();
    assert!((0..=2).contains(&arg_len));

    let obj = args.get(0);
    let is_err_arg = args.get(1);

    let mut is_err = false;
    if arg_len == 2 {
        let int_val = is_err_arg
            .integer_value(scope)
            .expect("Unable to convert to integer");
        is_err = int_val != 0;
    };
    let tc_scope = &mut v8::TryCatch::new(scope);
    let str_ = match obj.to_string(tc_scope) {
        Some(s) => s,
        None => v8::String::new(tc_scope, "").unwrap(),
    };
    if is_err {
        eprintln!("{}", str_.to_rust_string_lossy(tc_scope));
    } else {
        println!("{}", str_.to_rust_string_lossy(tc_scope));
    }
}

fn queue_microtask(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let obj = args.get(0);
    let func = v8::Local::<v8::Function>::try_from(obj).unwrap();
    scope.enqueue_microtask(func);
}

fn set_timeout(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    if args.length() == 0 {
        return;
    }
    let val = args.get(0);
    if !val.is_function() {
        let e = v8::String::new(scope, "Callback must be a function").unwrap();
        let error = v8::Exception::error(scope, e);
        scope.throw_exception(error);
    }

    let func = v8::Local::<v8::Function>::try_from(val).unwrap();
    if args.length() == 1 {
        scope.enqueue_microtask(func);
        return;
    }

    let delay_arg = args.get(1);
    if !delay_arg.is_number() {
        let e = v8::String::new(scope, "Delay must be a number").unwrap();
        let error = v8::Exception::error(scope, e);
        scope.throw_exception(error);
    }
    let global_func = v8::Global::new(scope, func);
    let delay = v8::Local::<v8::Number>::try_from(delay_arg).unwrap();

    let queue = scope.get_slot_mut::<TimerQueue>().unwrap();
    queue.timers.push(TimerEvent {
        call_at: epoch_millis() + delay.value() as u128,
        func: global_func,
        interval: None,
    })
}

struct TimerEvent {
    call_at: u128,
    func: v8::Global<v8::Function>,
    interval: Option<u64>,
}

struct TimerQueue {
    timers: Vec<TimerEvent>,
}

impl TimerQueue {
    fn new() -> Self {
        Self { timers: Vec::new() }
    }
}

fn epoch_millis() -> u128 {
    let start = SystemTime::now();
    let since_the_epoch = start.duration_since(UNIX_EPOCH).unwrap();
    since_the_epoch.as_millis()
}
