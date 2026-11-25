use super::*;

#[test]
fn captures_bytecode_locations_and_formats_them_lazily_once() {
    let (mut host, _) = host(false);
    let value = host.eval(r#"
        globalThis.captured = { name: 'CustomError', message: 'details' };
        function outer() { return inner(); }
        function inner() { Error.captureStackTrace(captured); }
        outer();
        let calls = 0;
        Error.prepareStackTrace = function(target, sites) {
          if (this !== Error || target !== captured) throw new Error('formatter arguments');
          calls++;
          return sites;
        };
        const sites = captured.stack;
        if (captured.stack !== sites || calls !== 1) throw new Error('cached formatter');
        delete Error.prepareStackTrace;
        const first = sites[0];
        JSON.stringify([first.getFileName(), first.getFunctionName(), sites[1].getFunctionName(),
          first.getLineNumber() > 0, first.getColumnNumber() > 0,
          first.getScriptNameOrSourceURL() === first.getFileName(),
          first.getThis() === undefined, first.getFunction() === undefined,
          first.getMethodName(), first.getTypeName(), first.getEvalOrigin() === undefined,
          first.getPromiseIndex(), first.isNative(), first.isEval(), first.isAsync(), first.isPromiseAll(),
          first.toString().includes('inner (C:/sources (工作)/caller.js:')]);
    "#, "C:/sources (工作)/caller.js").unwrap();
    assert_eq!(
        value
            .to_string(host.context())
            .unwrap()
            .to_std_string_escaped(),
        r#"["C:/sources (工作)/caller.js","inner","outer",true,true,true,true,true,null,null,true,null,false,false,false,false,true]"#
    );
}

#[test]
fn omits_the_requested_function_body_without_matching_names() {
    let (mut host, _) = host(true);
    assert_eq!(
        string(
            &mut host,
            r#"
      Error.prepareStackTrace = (_, sites) => sites.map(site => site.getFunctionName());
      function outside() { return hidden(); }
      function hidden() { const target={}; nested(target); return target.stack; }
      function nested(target) { Error.captureStackTrace(target, hidden); }
      const result = outside();
      const absent = {}; Error.captureStackTrace(absent, function notCalled(){});
      const native = {}; Error.captureStackTrace(native, Math.max);
      const sameName = function hidden(){ const target={}; Error.captureStackTrace(target, hidden); return target.stack; };
      const different = sameName();
      const unmatchedName = {hidden(){const target={}; Error.captureStackTrace(target, hidden); return target.stack;}}.hidden();
      const nonFunction={}; Error.captureStackTrace(nonFunction, {});
      JSON.stringify([result[0], !result.includes('hidden') && !result.includes('nested'),
        absent.stack.length, native.stack.length, different.length, unmatchedName.length, nonFunction.stack.length>0]);
    "#
        ),
        r#"["outside",true,0,0,1,0,true]"#
    );
}

#[test]
fn limits_are_per_context_and_stack_properties_are_configurable_and_writable() {
    let (mut host, _) = host(false);
    assert_eq!(
        string(
            &mut host,
            r#"
      function capture() { const result={}; Error.captureStackTrace(result); return result; }
      Error.prepareStackTrace = (_, sites) => sites;
      const lengths=[];
      for (const limit of [0, -1, NaN, '2', undefined, 1, 1.9, Infinity]) {
        Error.stackTraceLimit=limit; lengths.push(capture().stack.length);
      }
      const target=capture();
      const descriptor=Object.getOwnPropertyDescriptor(target,'stack');
      target.stack='replacement';
      const assigned=target.stack;
      delete target.stack;
      JSON.stringify([lengths.slice(0,7),lengths[7]>1, descriptor.enumerable, descriptor.configurable,
        typeof descriptor.get, typeof descriptor.set, assigned, Object.hasOwn(target,'stack'),
        Object.keys(Error).includes('captureStackTrace'), Object.keys(Error).includes('stackTraceLimit')]);
    "#
        ),
        r#"[[0,0,0,0,0,1,1],true,false,true,"function","function","replacement",false,false,false]"#
    );
    let (mut second, _) = super::host(false);
    assert_eq!(string(&mut second, "Error.stackTraceLimit"), "10");
}

#[test]
fn default_formatting_target_validation_and_formatter_failures() {
    let (mut host, _) = host(false);
    assert_eq!(
        string(
            &mut host,
            r#"
      const headings=[];
      Error.stackTraceLimit=0;
      for(const target of [{},{name:'Named'},{message:'text'},{name:'',message:'text'},{name:'',message:''},{name:null,message:0}]) {
        Error.captureStackTrace(target); headings.push(target.stack);
      }
      let invalid=0;
      for(const target of [null,undefined,1,'x',true,Symbol('x'),Object.freeze({})]) {
        try { Error.captureStackTrace(target); } catch(error) { if(error instanceof TypeError) invalid++; }
      }
      let calls=0;
      const target={}; Error.captureStackTrace(target);
      Error.prepareStackTrace=()=>{ calls++; if(calls===1) throw new Error('retry'); return undefined; };
      let failed=false; try { target.stack; } catch(error) { failed=error.message==='retry'; }
      const empty=target.stack; target.stack;
      delete Error.prepareStackTrace;
      Error.stackTraceLimit=10;
      const formatted={name:'Oops',message:'failure'}; Error.captureStackTrace(formatted);
      const functionTarget=()=>{}; Error.captureStackTrace(functionTarget); functionTarget.stack=42;
      JSON.stringify([headings,invalid,failed,empty===undefined,calls,
        formatted.stack.startsWith('Oops: failure\n    at '),functionTarget.stack]);
    "#
        ),
        r#"[["Error","Named","Error: text","text","","null: 0"],7,true,true,2,true,42]"#
    );
}

#[test]
fn a_formatter_can_read_the_same_stack_without_recursive_formatting() {
    let (mut host, _) = host(false);
    assert_eq!(
        string(
            &mut host,
            r#"
      const target={name:'Recursive'}; Error.captureStackTrace(target);
      let calls=0;
      Error.prepareStackTrace=(error,sites)=>{ calls++; return {fallback:error.stack,count:sites.length}; };
      const result=target.stack;
      JSON.stringify([calls,result.fallback.startsWith('Recursive\n    at '),result.count>0,target.stack===result]);
    "#
        ),
        "[1,true,true,true]"
    );
}

#[test]
fn anonymous_sources_and_empty_native_contexts_have_no_invented_locations() {
    let (mut host, _) = host(false);
    let value = host.context().eval(Source::from_bytes(r#"
      Error.prepareStackTrace=(_,sites)=>sites;
      globalThis.target={}; Error.captureStackTrace(target);
      JSON.stringify([target.stack[0].getFileName(), target.stack[0].getFunctionName(), target.stack[0].toString().includes('<anonymous>')]);
    "#)).unwrap();
    assert_eq!(
        value
            .to_string(host.context())
            .unwrap()
            .to_std_string_escaped(),
        "[null,null,true]"
    );
    let mut context = Context::default();
    let value = crate::stack::frames(&JsValue::undefined(), &[], &mut context).unwrap();
    assert_eq!(value.to_json(&mut context).unwrap(), Some(json!([])));
}

#[test]
fn package_call_sites_agree_with_module_filenames_and_find_the_actual_importer() {
    let (mut host, _) = host(true);
    let binding = module(
        "deps/bindings.js",
        r#"
      exports.getFileName = function getFileName() {
        const previous=Error.prepareStackTrace;
        const target={};
        let file;
        Error.prepareStackTrace=(_,sites)=>{
          for(const site of sites) {
            file=site.getFileName();
            if(file!==__filename) return;
          }
        };
        Error.captureStackTrace(target); target.stack;
        Error.prepareStackTrace=previous;
        return file;
      };
    "#,
        &[],
    );
    let caller = module(
        "original/main.ts",
        r#"
      const binding=require('./binding');
      globalThis.callerPath=binding.getFileName();
      globalThis.modulePath=__filename;
    "#,
        &[("./binding", Resolution::Module(1))],
    );
    let mut bundle = script(vec![caller, binding], vec![0]);
    bundle.bundle.modules[0].runtime_path = Some("out/main.js".into());
    host.load(&bundle, &[], "C:/installed/app/").unwrap();
    assert_eq!(
        string(&mut host, "JSON.stringify([callerPath,modulePath])"),
        r#"["C:/installed/app/out/main.js","C:/installed/app/out/main.js"]"#
    );
}
