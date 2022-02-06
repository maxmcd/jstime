use rand::prelude::*;
use std::convert::TryFrom;
use std::iter::IntoIterator;
use std::sync::mpsc::channel;
use std::sync::mpsc::{Receiver, Sender};
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
                function: v8::MapFnTo::map_fn_to(fetch),
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
        binding!("fetch", fetch);
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
        builtin!("./fetch.js");
        builtin!("./performance.js");
        builtin!("./encoders.js");
        builtin!("./queue_microtask.js");
    }
    pub(crate) fn init(scope: &mut v8::HandleScope) {
        scope.set_slot(TimerQueue::new());
        scope.set_slot(Instant::now() as TimeOrigin);

        let (send, recv) = channel();
        let (send2, recv2) = channel();

        std::thread::spawn(move || loop {
            let req: RequestRequest = recv2.recv().unwrap();

            send.send(RequestResponse {
                id: req.id,
                value: req.value.call(),
            })
            .unwrap();
        });

        scope.set_slot(Context {
            tq: TimerQueue::new(),
            outstanding_promises: std::collections::HashMap::new(),
            promise_counter: 0,
            response_receiver: recv,
            request_sender: send2,
        });
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

fn exception(scope: &mut v8::HandleScope, err: &str) {
    let e = v8::String::new(scope, err).unwrap();
    let error = v8::Exception::error(scope, e);
    scope.throw_exception(error);
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
        exception(scope, "Callback must be a function");
        return;
    }

    let func = v8::Local::<v8::Function>::try_from(val).unwrap();
    if args.length() == 1 {
        scope.enqueue_microtask(func);
        return;
    }

    let delay_arg = args.get(1);
    if !delay_arg.is_number() {
        exception(scope, "Delay must be a number");
        return;
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
    fn empty(&self) -> bool {
        self.timers.len() == 0
    }
}

fn epoch_millis() -> u128 {
    let start = SystemTime::now();
    let since_the_epoch = start.duration_since(UNIX_EPOCH).unwrap();
    since_the_epoch.as_millis()
}

fn fetch(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    if args.length() == 0 {
        return exception(scope, "1 argument required, but only 0 present");
    }
    let resource = args.get(0);
    if !resource.is_string() {
        return exception(scope, "first argument to fetch must be a string");
    }
    let method = "GET";
    let headers: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if args.length() >= 2 {
        let init = args.get(1);
        if !init.is_object() {
            return exception(scope, "fetch init argument must be an object");
        }
        let options = v8::Local::<v8::Object>::try_from(init).unwrap();
        let headers_key = v8::String::new(scope, "headers").unwrap();
        if let Some(headers_local) = options.get(scope, headers_key.into()) {
            if !headers_local.is_object() {
                return exception(scope, "headers must be an object");
            }
            let headers_val = v8::Local::<v8::Object>::try_from(headers_local).unwrap();
            let _names = headers_val.get_property_names(scope).unwrap();
            // TODO: complete
        }
        // TODO: complete
        // let method_key = v8::String::new(scope, "method").unwrap();
        // if let Some(method_local) = options.get(scope, headers_key.into()) {
        //     if method_local.is_string() {
        //         let method_val = v8::Local::<v8::String>::try_from(method_local).unwrap();
        //         let m_string: String = method_val.to_rust_string_lossy(scope).to_owned();
        //         method = &*m_string;
        //     }
        // }
    }

    let resolver = v8::PromiseResolver::new(scope).unwrap();
    let global_promise = v8::Global::new(scope, resolver);
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let resource = &resource.to_rust_string_lossy(scope).to_owned();
    let ctx = scope.get_slot_mut::<Context>().unwrap();
    ctx.fetch(global_promise, ureq::request(method, resource));
}

struct Context {
    tq: TimerQueue,
    outstanding_promises: std::collections::HashMap<u32, v8::Global<v8::PromiseResolver>>,
    promise_counter: u32,
    response_receiver: Receiver<RequestResponse>,
    request_sender: Sender<RequestRequest>,
}

impl Context {
    fn fetch(&mut self, pr: v8::Global<v8::PromiseResolver>, req: ureq::Request) {
        self.promise_counter += 1;
        self.request_sender
            .send(RequestRequest {
                id: self.promise_counter,
                value: req,
            })
            .unwrap();
        self.outstanding_promises.insert(self.promise_counter, pr);
    }
}

struct RequestRequest {
    id: u32,
    value: ureq::Request,
}

struct RequestResponse {
    id: u32,
    value: Result<ureq::Response, ureq::Error>,
}

pub fn tick(scope: &mut v8::HandleScope) -> bool {
    let ctx = scope.get_slot_mut::<Context>().unwrap();
    let no_promises = ctx.outstanding_promises.len() == 0;
    let no_timers = ctx.tq.empty();
    if no_promises && no_timers {
        return false;
    }
    if no_promises {
        // TODO: sleep until next timer and then add microtask
        return false;
    }
    println!("possible promise");
    let possible_promise = if no_timers {
        Some(ctx.response_receiver.recv().unwrap())
    } else {
        match ctx
            .response_receiver
            .recv_timeout(std::time::Duration::from_millis(100))
        {
            Ok(v) => Some(v),
            Err(_) => None,
        }
    };
    if let Some(result) = possible_promise {
        let resolver_global = ctx.outstanding_promises.remove(&result.id).unwrap();
        let resolver = resolver_global.open(scope);
        let status_code = v8::Number::new(scope, result.value.unwrap().status() as f64);
        resolver.resolve(scope, status_code.into());
    }
    true
}
