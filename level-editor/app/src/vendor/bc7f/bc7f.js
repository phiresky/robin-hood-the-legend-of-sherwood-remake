// This code implements the `-sMODULARIZE` settings by taking the generated
// JS program code (INNER_JS_CODE) and wrapping it in a factory function.

// When targetting node and ES6 we use `await import ..` in the generated code
// so the outer function needs to be marked as async.
async function BC7F(moduleArg = {}) {
  var moduleRtn;

  // include: shell.js
  // include: minimum_runtime_check.js
  // end include: minimum_runtime_check.js
  // The Module object: Our interface to the outside world. We import
  // and export values on it. There are various ways Module can be used:
  // 1. Not defined. We create it here
  // 2. A function parameter, function(moduleArg) => Promise<Module>
  // 3. pre-run appended it, var Module = {}; ..generated code..
  // 4. External script tag defines var Module.
  // We need to check if Module already exists (e.g. case 3 above).
  // Substitution will be replaced with actual code on later stage of the build,
  // this way Closure Compiler will not mangle it (e.g. case 4. above).
  // Note that if you want to run closure, and also to use Module
  // after the generated code, you will need to define   var Module = {};
  // before the code. Then that object will be used in the code, and you
  // can continue to use Module afterwards as well.
  var Module = moduleArg;

  // Determine the runtime environment we are in. You can customize this by
  // setting the ENVIRONMENT setting at compile time (see settings.js).

  var ENVIRONMENT_IS_WEB = false;
  var ENVIRONMENT_IS_WORKER = true;
  var ENVIRONMENT_IS_NODE = false;
  var ENVIRONMENT_IS_SHELL = false;

  // --pre-jses are emitted after the Module integration code, so that they can
  // refer to Module (if they choose; they can also define Module)

  var arguments_ = [];
  var thisProgram = "./this.program";
  var quit_ = (status, toThrow) => {
    throw toThrow;
  };

  var _scriptName = import.meta.url;

  // `/` should be present at the end if `scriptDirectory` is not empty
  var scriptDirectory = "";
  function locateFile(path) {
    if (Module["locateFile"]) {
      return Module["locateFile"](path, scriptDirectory);
    }
    return scriptDirectory + path;
  }

  // Hooks that are implemented differently in different runtime environments.
  var readAsync, readBinary;

  // Note that this includes Node.js workers when relevant (pthreads is enabled).
  // Node.js workers are detected as a combination of ENVIRONMENT_IS_WORKER and
  // ENVIRONMENT_IS_NODE.
  if (ENVIRONMENT_IS_WEB || ENVIRONMENT_IS_WORKER) {
    try {
      scriptDirectory = new URL(".", _scriptName).href; // includes trailing slash
    } catch {
      // Must be a `blob:` or `data:` URL (e.g. `blob:http://site.com/etc/etc`), we cannot
      // infer anything from them.
    }

    {
      // include: web_or_worker_shell_read.js
      if (ENVIRONMENT_IS_WORKER) {
        readBinary = (url) => {
          var xhr = new XMLHttpRequest();
          xhr.open("GET", url, false);
          xhr.responseType = "arraybuffer";
          xhr.send(null);
          return new Uint8Array(/** @type{!ArrayBuffer} */ (xhr.response));
        };
      }

      readAsync = async (url) => {
        var response = await fetch(url, { credentials: "same-origin" });
        if (response.ok) {
          return response.arrayBuffer();
        }
        throw new Error(response.status + " : " + response.url);
      };
      // end include: web_or_worker_shell_read.js
    }
  } else {
  }

  var out = console.log.bind(console);
  var err = console.error.bind(console);

  // end include: shell.js

  // include: preamble.js
  // === Preamble library stuff ===

  // Documentation for the public APIs defined in this file must be updated in:
  //    site/source/docs/api_reference/preamble.js.rst
  // A prebuilt local version of the documentation is available at:
  //    site/build/text/docs/api_reference/preamble.js.txt
  // You can also build docs locally as HTML or other formats in site/
  // An online HTML version (which may be of a different version of Emscripten)
  //    is up at http://kripken.github.io/emscripten-site/docs/api_reference/preamble.js.html

  var wasmBinary;

  // Wasm globals

  //========================================
  // Runtime essentials
  //========================================

  // whether we are quitting the application. no code should run after this.
  // set in exit() and abort()
  var ABORT = false;

  // set by exit() and abort().  Passed to 'onExit' handler.
  // NOTE: This is also used as the process return code code in shell environments
  // but only when noExitRuntime is false.
  var EXITSTATUS;

  // In STRICT mode, we only define assert() when ASSERTIONS is set.  i.e. we
  // don't define it at all in release modes.  This matches the behaviour of
  // MINIMAL_RUNTIME.
  // TODO(sbc): Make this the default even without STRICT enabled.
  /** @type {function(*, string=)} */
  function assert(condition, text) {
    if (!condition) {
      // This build was created without ASSERTIONS defined.  `assert()` should not
      // ever be called in this configuration but in case there are callers in
      // the wild leave this simple abort() implementation here for now.
      abort(text);
    }
  }

  /**
   * Indicates whether filename is delivered via file protocol (as opposed to http/https)
   * @noinline
   */
  var isFileURI = (filename) => filename.startsWith("file://");

  // include: runtime_common.js
  // include: runtime_stack_check.js
  // end include: runtime_stack_check.js
  // include: runtime_exceptions.js
  // end include: runtime_exceptions.js
  // include: runtime_debug.js
  // end include: runtime_debug.js
  var readyPromiseResolve, readyPromiseReject;

  // Memory management
  var /** @type {!Int8Array} */
    HEAP8,
    /** @type {!Uint8Array} */
    HEAPU8,
    /** @type {!Int16Array} */
    HEAP16,
    /** @type {!Uint16Array} */
    HEAPU16,
    /** @type {!Int32Array} */
    HEAP32,
    /** @type {!Uint32Array} */
    HEAPU32,
    /** @type {!Float32Array} */
    HEAPF32,
    /** @type {!Float64Array} */
    HEAPF64;

  // BigInt64Array type is not correctly defined in closure
  var /** not-@type {!BigInt64Array} */
    HEAP64,
    /* BigUint64Array type is not correctly defined in closure
/** not-@type {!BigUint64Array} */
    HEAPU64;

  var runtimeInitialized = false;

  function updateMemoryViews() {
    var b = wasmMemory.buffer;
    HEAP8 = new Int8Array(b);
    HEAP16 = new Int16Array(b);
    Module["HEAPU8"] = HEAPU8 = new Uint8Array(b);
    HEAPU16 = new Uint16Array(b);
    HEAP32 = new Int32Array(b);
    HEAPU32 = new Uint32Array(b);
    HEAPF32 = new Float32Array(b);
    HEAPF64 = new Float64Array(b);
    HEAP64 = new BigInt64Array(b);
    HEAPU64 = new BigUint64Array(b);
  }

  // include: memoryprofiler.js
  // end include: memoryprofiler.js
  // end include: runtime_common.js
  function preRun() {
    if (Module["preRun"]) {
      if (typeof Module["preRun"] == "function") Module["preRun"] = [Module["preRun"]];
      while (Module["preRun"].length) {
        addOnPreRun(Module["preRun"].shift());
      }
    }
    // Begin ATPRERUNS hooks
    callRuntimeCallbacks(onPreRuns);
    // End ATPRERUNS hooks
  }

  function initRuntime() {
    runtimeInitialized = true;

    // No ATINITS hooks

    wasmExports["__wasm_call_ctors"]();

    // No ATPOSTCTORS hooks
  }

  function postRun() {
    // PThreads reuse the runtime from the main thread.

    if (Module["postRun"]) {
      if (typeof Module["postRun"] == "function") Module["postRun"] = [Module["postRun"]];
      while (Module["postRun"].length) {
        addOnPostRun(Module["postRun"].shift());
      }
    }

    // Begin ATPOSTRUNS hooks
    callRuntimeCallbacks(onPostRuns);
    // End ATPOSTRUNS hooks
  }

  /** @param {string|number=} what */
  function abort(what) {
    Module["onAbort"]?.(what);

    what = "Aborted(" + what + ")";
    // TODO(sbc): Should we remove printing and leave it up to whoever
    // catches the exception?
    err(what);

    ABORT = true;

    what += ". Build with -sASSERTIONS for more info.";

    // Use a wasm runtime error, because a JS error might be seen as a foreign
    // exception, which means we'd run destructors on it. We need the error to
    // simply make the program stop.
    // FIXME This approach does not work in Wasm EH because it currently does not assume
    // all RuntimeErrors are from traps; it decides whether a RuntimeError is from
    // a trap or not based on a hidden field within the object. So at the moment
    // we don't have a way of throwing a wasm trap from JS. TODO Make a JS API that
    // allows this in the wasm spec.

    // Suppress closure compiler warning here. Closure compiler's builtin extern
    // definition for WebAssembly.RuntimeError claims it takes no arguments even
    // though it can.
    // TODO(https://github.com/google/closure-compiler/pull/3913): Remove if/when upstream closure gets fixed.
    /** @suppress {checkTypes} */
    var e = new WebAssembly.RuntimeError(what);

    readyPromiseReject?.(e);
    // Throw the error whether or not MODULARIZE is set because abort is used
    // in code paths apart from instantiation where an exception is expected
    // to be thrown when abort is called.
    throw e;
  }

  var wasmBinaryFile;

  function findWasmBinary() {
    if (Module["locateFile"]) {
      return locateFile("bc7f.wasm");
    }

    // Use bundler-friendly `new URL(..., import.meta.url)` pattern; works in browsers too.
    return new URL("bc7f.wasm", import.meta.url).href;
  }

  function getBinarySync(file) {
    if (file == wasmBinaryFile && wasmBinary) {
      return new Uint8Array(wasmBinary);
    }
    if (readBinary) {
      return readBinary(file);
    }
    // Throwing a plain string here, even though it not normally adviables since
    // this gets turning into an `abort` in instantiateArrayBuffer.
    throw "both async and sync fetching of the wasm failed";
  }

  async function getWasmBinary(binaryFile) {
    // If we don't have the binary yet, load it asynchronously using readAsync.
    if (!wasmBinary) {
      // Fetch the binary using readAsync
      try {
        var response = await readAsync(binaryFile);
        return new Uint8Array(response);
      } catch {
        // Fall back to getBinarySync below;
      }
    }

    // Otherwise, getBinarySync should be able to get it synchronously
    return getBinarySync(binaryFile);
  }

  async function instantiateArrayBuffer(binaryFile, imports) {
    try {
      var binary = await getWasmBinary(binaryFile);
      var instance = await WebAssembly.instantiate(binary, imports);
      return instance;
    } catch (reason) {
      err(`failed to asynchronously prepare wasm: ${reason}`);

      abort(reason);
    }
  }

  async function instantiateAsync(binary, binaryFile, imports) {
    if (!binary) {
      try {
        var response = fetch(binaryFile, { credentials: "same-origin" });
        var instantiationResult = await WebAssembly.instantiateStreaming(response, imports);
        return instantiationResult;
      } catch (reason) {
        // We expect the most common failure cause to be a bad MIME type for the binary,
        // in which case falling back to ArrayBuffer instantiation should work.
        err(`wasm streaming compile failed: ${reason}`);
        err("falling back to ArrayBuffer instantiation");
        // fall back of instantiateArrayBuffer below
      }
    }
    return instantiateArrayBuffer(binaryFile, imports);
  }

  function getWasmImports() {
    // prepare imports
    var imports = {
      env: wasmImports,
      wasi_snapshot_preview1: wasmImports,
    };
    return imports;
  }

  // Create the wasm instance.
  // Receives the wasm imports, returns the exports.
  async function createWasm() {
    // Load the wasm module and create an instance of using native support in the JS engine.
    // handle a generated wasm instance, receiving its exports and
    // performing other necessary setup
    /** @param {WebAssembly.Module=} module*/
    function receiveInstance(instance, module) {
      wasmExports = instance.exports;

      assignWasmExports(wasmExports);

      updateMemoryViews();

      return wasmExports;
    }

    // Prefer streaming instantiation if available.
    function receiveInstantiationResult(result) {
      // 'result' is a ResultObject object which has both the module and instance.
      // receiveInstance() will swap in the exports (to Module.asm) so they can be called
      // TODO: Due to Closure regression https://github.com/google/closure-compiler/issues/3193, the above line no longer optimizes out down to the following line.
      // When the regression is fixed, can restore the above PTHREADS-enabled path.
      return receiveInstance(result["instance"]);
    }

    var info = getWasmImports();

    // User shell pages can write their own Module.instantiateWasm = function(imports, successCallback) callback
    // to manually instantiate the Wasm module themselves. This allows pages to
    // run the instantiation parallel to any other async startup actions they are
    // performing.
    // Also pthreads and wasm workers initialize the wasm instance through this
    // path.
    if (Module["instantiateWasm"]) {
      return new Promise((resolve, reject) => {
        Module["instantiateWasm"](info, (inst, mod) => {
          resolve(receiveInstance(inst, mod));
        });
      });
    }

    wasmBinaryFile ??= findWasmBinary();
    var result = await instantiateAsync(wasmBinary, wasmBinaryFile, info);
    var exports = receiveInstantiationResult(result);
    return exports;
  }

  // end include: preamble.js

  // Begin JS library code

  class ExitStatus {
    name = "ExitStatus";
    constructor(status) {
      this.message = `Program terminated with exit(${status})`;
      this.status = status;
    }
  }

  var callRuntimeCallbacks = (callbacks) => {
    while (callbacks.length > 0) {
      // Pass the module as the first argument.
      callbacks.shift()(Module);
    }
  };
  var onPostRuns = [];
  var addOnPostRun = (cb) => onPostRuns.push(cb);

  var onPreRuns = [];
  var addOnPreRun = (cb) => onPreRuns.push(cb);

  /**
   * @param {number} ptr
   * @param {string} type
   */
  function getValue(ptr, type = "i8") {
    if (type.endsWith("*")) type = "*";
    switch (type) {
      case "i1":
        return HEAP8[ptr];
      case "i8":
        return HEAP8[ptr];
      case "i16":
        return HEAP16[ptr >> 1];
      case "i32":
        return HEAP32[ptr >> 2];
      case "i64":
        return HEAP64[ptr >> 3];
      case "float":
        return HEAPF32[ptr >> 2];
      case "double":
        return HEAPF64[ptr >> 3];
      case "*":
        return HEAPU32[ptr >> 2];
      default:
        abort(`invalid type for getValue: ${type}`);
    }
  }

  var noExitRuntime = true;

  /**
   * @param {number} ptr
   * @param {number} value
   * @param {string} type
   */
  function setValue(ptr, value, type = "i8") {
    if (type.endsWith("*")) type = "*";
    switch (type) {
      case "i1":
        HEAP8[ptr] = value;
        break;
      case "i8":
        HEAP8[ptr] = value;
        break;
      case "i16":
        HEAP16[ptr >> 1] = value;
        break;
      case "i32":
        HEAP32[ptr >> 2] = value;
        break;
      case "i64":
        HEAP64[ptr >> 3] = BigInt(value);
        break;
      case "float":
        HEAPF32[ptr >> 2] = value;
        break;
      case "double":
        HEAPF64[ptr >> 3] = value;
        break;
      case "*":
        HEAPU32[ptr >> 2] = value;
        break;
      default:
        abort(`invalid type for setValue: ${type}`);
    }
  }

  var stackRestore = (val) => __emscripten_stack_restore(val);

  var stackSave = () => _emscripten_stack_get_current();

  var __abort_js = () => abort("");

  var runtimeKeepaliveCounter = 0;
  var __emscripten_runtime_keepalive_clear = () => {
    noExitRuntime = false;
    runtimeKeepaliveCounter = 0;
  };

  var timers = {};

  var handleException = (e) => {
    // Certain exception types we do not treat as errors since they are used for
    // internal control flow.
    // 1. ExitStatus, which is thrown by exit()
    // 2. "unwind", which is thrown by emscripten_unwind_to_js_event_loop() and others
    //    that wish to return to JS event loop.
    if (e instanceof ExitStatus || e == "unwind") {
      return EXITSTATUS;
    }
    quit_(1, e);
  };

  var keepRuntimeAlive = () => noExitRuntime || runtimeKeepaliveCounter > 0;
  var _proc_exit = (code) => {
    EXITSTATUS = code;
    if (!keepRuntimeAlive()) {
      Module["onExit"]?.(code);
      ABORT = true;
    }
    quit_(code, new ExitStatus(code));
  };
  /** @param {boolean|number=} implicit */
  var exitJS = (status, implicit) => {
    EXITSTATUS = status;

    _proc_exit(status);
  };
  var _exit = exitJS;

  var maybeExit = () => {
    if (!keepRuntimeAlive()) {
      try {
        _exit(EXITSTATUS);
      } catch (e) {
        handleException(e);
      }
    }
  };
  var callUserCallback = (func) => {
    if (ABORT) {
      return;
    }
    try {
      func();
      maybeExit();
    } catch (e) {
      handleException(e);
    }
  };

  var _emscripten_get_now = () => performance.now();
  var __setitimer_js = (which, timeout_ms) => {
    // First, clear any existing timer.
    if (timers[which]) {
      clearTimeout(timers[which].id);
      delete timers[which];
    }

    // A timeout of zero simply cancels the current timeout so we have nothing
    // more to do.
    if (!timeout_ms) return 0;

    var id = setTimeout(() => {
      delete timers[which];
      callUserCallback(() => __emscripten_timeout(which, _emscripten_get_now()));
    }, timeout_ms);
    timers[which] = { id, timeout_ms };
    return 0;
  };

  var getHeapMax = () =>
    // Stay one Wasm page short of 4GB: while e.g. Chrome is able to allocate
    // full 4GB Wasm memories, the size will wrap back to 0 bytes in Wasm side
    // for any code that deals with heap sizes, which would require special
    // casing all heap size related code to treat 0 specially.
    536870912;

  var alignMemory = (size, alignment) => {
    return Math.ceil(size / alignment) * alignment;
  };

  var growMemory = (size) => {
    var oldHeapSize = wasmMemory.buffer.byteLength;
    var pages = ((size - oldHeapSize + 65535) / 65536) | 0;
    try {
      // round size grow request up to wasm page size (fixed 64KB per spec)
      wasmMemory.grow(pages); // .grow() takes a delta compared to the previous size
      updateMemoryViews();
      return 1; /*success*/
    } catch (e) {}
    // implicit 0 return to save code size (caller will cast "undefined" into 0
    // anyhow)
  };
  var _emscripten_resize_heap = (requestedSize) => {
    var oldSize = HEAPU8.length;
    // With CAN_ADDRESS_2GB or MEMORY64, pointers are already unsigned.
    requestedSize >>>= 0;
    // With multithreaded builds, races can happen (another thread might increase the size
    // in between), so return a failure, and let the caller retry.

    // Memory resize rules:
    // 1.  Always increase heap size to at least the requested size, rounded up
    //     to next page multiple.
    // 2a. If MEMORY_GROWTH_LINEAR_STEP == -1, excessively resize the heap
    //     geometrically: increase the heap size according to
    //     MEMORY_GROWTH_GEOMETRIC_STEP factor (default +20%), At most
    //     overreserve by MEMORY_GROWTH_GEOMETRIC_CAP bytes (default 96MB).
    // 2b. If MEMORY_GROWTH_LINEAR_STEP != -1, excessively resize the heap
    //     linearly: increase the heap size by at least
    //     MEMORY_GROWTH_LINEAR_STEP bytes.
    // 3.  Max size for the heap is capped at 2048MB-WASM_PAGE_SIZE, or by
    //     MAXIMUM_MEMORY, or by ASAN limit, depending on which is smallest
    // 4.  If we were unable to allocate as much memory, it may be due to
    //     over-eager decision to excessively reserve due to (3) above.
    //     Hence if an allocation fails, cut down on the amount of excess
    //     growth, in an attempt to succeed to perform a smaller allocation.

    // A limit is set for how much we can grow. We should not exceed that
    // (the wasm binary specifies it, so if we tried, we'd fail anyhow).
    var maxHeapSize = getHeapMax();
    if (requestedSize > maxHeapSize) {
      return false;
    }

    // Loop through potential heap size increases. If we attempt a too eager
    // reservation that fails, cut down on the attempted size and reserve a
    // smaller bump instead. (max 3 times, chosen somewhat arbitrarily)
    for (var cutDown = 1; cutDown <= 4; cutDown *= 2) {
      var overGrownHeapSize = oldSize * (1 + 0.2 / cutDown); // ensure geometric growth
      // but limit overreserving (default to capping at +96MB overgrowth at most)
      overGrownHeapSize = Math.min(overGrownHeapSize, requestedSize + 100663296);

      var newSize = Math.min(
        maxHeapSize,
        alignMemory(Math.max(requestedSize, overGrownHeapSize), 65536),
      );

      var replacement = growMemory(newSize);
      if (replacement) {
        return true;
      }
    }
    return false;
  };

  var _fd_close = (fd) => {
    return 52;
  };

  var INT53_MAX = 9007199254740992;

  var INT53_MIN = -9007199254740992;
  var bigintToI53Checked = (num) => (num < INT53_MIN || num > INT53_MAX ? NaN : Number(num));
  function _fd_seek(fd, offset, whence, newOffset) {
    offset = bigintToI53Checked(offset);

    return 70;
  }

  var printCharBuffers = [null, [], []];

  var UTF8Decoder = globalThis.TextDecoder && new TextDecoder();

  var findStringEnd = (heapOrArray, idx, maxBytesToRead, ignoreNul) => {
    var maxIdx = idx + maxBytesToRead;
    if (ignoreNul) return maxIdx;
    // TextDecoder needs to know the byte length in advance, it doesn't stop on
    // null terminator by itself.
    // As a tiny code save trick, compare idx against maxIdx using a negation,
    // so that maxBytesToRead=undefined/NaN means Infinity.
    while (heapOrArray[idx] && !(idx >= maxIdx)) ++idx;
    return idx;
  };

  /**
   * Given a pointer 'idx' to a null-terminated UTF8-encoded string in the given
   * array that contains uint8 values, returns a copy of that string as a
   * Javascript String object.
   * heapOrArray is either a regular array, or a JavaScript typed array view.
   * @param {number=} idx
   * @param {number=} maxBytesToRead
   * @param {boolean=} ignoreNul - If true, the function will not stop on a NUL character.
   * @return {string}
   */
  var UTF8ArrayToString = (heapOrArray, idx = 0, maxBytesToRead, ignoreNul) => {
    var endPtr = findStringEnd(heapOrArray, idx, maxBytesToRead, ignoreNul);

    // When using conditional TextDecoder, skip it for short strings as the overhead of the native call is not worth it.
    if (endPtr - idx > 16 && heapOrArray.buffer && UTF8Decoder) {
      return UTF8Decoder.decode(heapOrArray.subarray(idx, endPtr));
    }
    var str = "";
    while (idx < endPtr) {
      // For UTF8 byte structure, see:
      // http://en.wikipedia.org/wiki/UTF-8#Description
      // https://www.ietf.org/rfc/rfc2279.txt
      // https://tools.ietf.org/html/rfc3629
      var u0 = heapOrArray[idx++];
      if (!(u0 & 0x80)) {
        str += String.fromCharCode(u0);
        continue;
      }
      var u1 = heapOrArray[idx++] & 63;
      if ((u0 & 0xe0) == 0xc0) {
        str += String.fromCharCode(((u0 & 31) << 6) | u1);
        continue;
      }
      var u2 = heapOrArray[idx++] & 63;
      if ((u0 & 0xf0) == 0xe0) {
        u0 = ((u0 & 15) << 12) | (u1 << 6) | u2;
      } else {
        u0 = ((u0 & 7) << 18) | (u1 << 12) | (u2 << 6) | (heapOrArray[idx++] & 63);
      }

      if (u0 < 0x10000) {
        str += String.fromCharCode(u0);
      } else {
        var ch = u0 - 0x10000;
        str += String.fromCharCode(0xd800 | (ch >> 10), 0xdc00 | (ch & 0x3ff));
      }
    }
    return str;
  };
  var printChar = (stream, curr) => {
    var buffer = printCharBuffers[stream];
    if (curr === 0 || curr === 10) {
      (stream === 1 ? out : err)(UTF8ArrayToString(buffer));
      buffer.length = 0;
    } else {
      buffer.push(curr);
    }
  };

  var flush_NO_FILESYSTEM = () => {
    // flush anything remaining in the buffers during shutdown
    if (printCharBuffers[1].length) printChar(1, 10);
    if (printCharBuffers[2].length) printChar(2, 10);
  };

  /**
   * Given a pointer 'ptr' to a null-terminated UTF8-encoded string in the
   * emscripten HEAP, returns a copy of that string as a Javascript String object.
   *
   * @param {number} ptr
   * @param {number=} maxBytesToRead - An optional length that specifies the
   *   maximum number of bytes to read. You can omit this parameter to scan the
   *   string until the first 0 byte. If maxBytesToRead is passed, and the string
   *   at [ptr, ptr+maxBytesToReadr[ contains a null byte in the middle, then the
   *   string will cut short at that byte index.
   * @param {boolean=} ignoreNul - If true, the function will not stop on a NUL character.
   * @return {string}
   */
  var UTF8ToString = (ptr, maxBytesToRead, ignoreNul) => {
    return ptr ? UTF8ArrayToString(HEAPU8, ptr, maxBytesToRead, ignoreNul) : "";
  };
  var SYSCALLS = {
    varargs: undefined,
    getStr(ptr) {
      var ret = UTF8ToString(ptr);
      return ret;
    },
  };
  var _fd_write = (fd, iov, iovcnt, pnum) => {
    // hack to support printf in SYSCALLS_REQUIRE_FILESYSTEM=0
    var num = 0;
    for (var i = 0; i < iovcnt; i++) {
      var ptr = HEAPU32[iov >> 2];
      var len = HEAPU32[(iov + 4) >> 2];
      iov += 8;
      for (var j = 0; j < len; j++) {
        printChar(fd, HEAPU8[ptr + j]);
      }
      num += len;
    }
    HEAPU32[pnum >> 2] = num;
    return 0;
  };

  // End JS library code

  // include: postlibrary.js
  // This file is included after the automatically-generated JS library code
  // but before the wasm module is created.

  {
    // Begin ATMODULES hooks
    if (Module["noExitRuntime"]) noExitRuntime = Module["noExitRuntime"];
    if (Module["print"]) out = Module["print"];
    if (Module["printErr"]) err = Module["printErr"];
    if (Module["wasmBinary"]) wasmBinary = Module["wasmBinary"];
    // End ATMODULES hooks

    if (Module["arguments"]) arguments_ = Module["arguments"];
    if (Module["thisProgram"]) thisProgram = Module["thisProgram"];

    if (Module["preInit"]) {
      if (typeof Module["preInit"] == "function") Module["preInit"] = [Module["preInit"]];
      while (Module["preInit"].length > 0) {
        Module["preInit"].shift()();
      }
    }
  }

  // Begin runtime exports
  // End runtime exports
  // Begin JS library exports
  // End JS library exports

  // end include: postlibrary.js

  // Imports from the Wasm binary.
  var _initialize,
    _encode,
    _mip,
    _malloc,
    _free,
    __emscripten_timeout,
    __emscripten_stack_restore,
    __emscripten_stack_alloc,
    _emscripten_stack_get_current,
    memory,
    __indirect_function_table,
    wasmMemory;

  function assignWasmExports(wasmExports) {
    _initialize = Module["_initialize"] = wasmExports["initialize"];
    _encode = Module["_encode"] = wasmExports["encode"];
    _mip = Module["_mip"] = wasmExports["mip"];
    _malloc = Module["_malloc"] = wasmExports["malloc"];
    _free = Module["_free"] = wasmExports["free"];
    __emscripten_timeout = wasmExports["_emscripten_timeout"];
    __emscripten_stack_restore = wasmExports["_emscripten_stack_restore"];
    __emscripten_stack_alloc = wasmExports["_emscripten_stack_alloc"];
    _emscripten_stack_get_current = wasmExports["emscripten_stack_get_current"];
    memory = wasmMemory = wasmExports["memory"];
    __indirect_function_table = wasmExports["__indirect_function_table"];
  }

  var wasmImports = {
    /** @export */
    _abort_js: __abort_js,
    /** @export */
    _emscripten_runtime_keepalive_clear: __emscripten_runtime_keepalive_clear,
    /** @export */
    _setitimer_js: __setitimer_js,
    /** @export */
    emscripten_resize_heap: _emscripten_resize_heap,
    /** @export */
    fd_close: _fd_close,
    /** @export */
    fd_seek: _fd_seek,
    /** @export */
    fd_write: _fd_write,
    /** @export */
    proc_exit: _proc_exit,
  };

  // include: postamble.js
  // === Auto-generated postamble setup entry stuff ===

  function run() {
    preRun();

    function doRun() {
      // run may have just been called through dependencies being fulfilled just in this very frame,
      // or while the async setStatus time below was happening
      Module["calledRun"] = true;

      if (ABORT) return;

      initRuntime();

      readyPromiseResolve?.(Module);
      Module["onRuntimeInitialized"]?.();

      postRun();
    }

    if (Module["setStatus"]) {
      Module["setStatus"]("Running...");
      setTimeout(() => {
        setTimeout(() => Module["setStatus"](""), 1);
        doRun();
      }, 1);
    } else {
      doRun();
    }
  }

  var wasmExports;

  // In modularize mode the generated code is within a factory function so we
  // can use await here (since it's not top-level-await).
  wasmExports = await createWasm();

  run();

  // end include: postamble.js

  // include: postamble_modularize.js
  // In MODULARIZE mode we wrap the generated code in a factory function
  // and return either the Module itself, or a promise of the module.
  //
  // We assign to the `moduleRtn` global here and configure closure to see
  // this as and extern so it won't get minified.

  if (runtimeInitialized) {
    moduleRtn = Module;
  } else {
    // Set up the promise that indicates the Module is initialized
    moduleRtn = new Promise((resolve, reject) => {
      readyPromiseResolve = resolve;
      readyPromiseReject = reject;
    });
  }

  // end include: postamble_modularize.js

  return moduleRtn;
}

// Export using a UMD style export, or ES6 exports if selected
export default BC7F;
