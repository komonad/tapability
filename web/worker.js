// Runs the WebAssembly engine off the main thread.
//
// The page never calls into WebAssembly directly: it posts `{ id, cmd }` here
// and gets `{ id, state }` back. Because the worker is single threaded and
// postMessage preserves order, replies always arrive in the order the commands
// were sent - the page can fire a whole drag stroke without waiting.
//
// The module is loaded by hand (no wasm-bindgen): the engine exposes a scratch
// buffer for the command text and a JSON reply buffer, both in linear memory.

const imports = {
  env: {
    // tapa-core::clock calls this for monotonic milliseconds; there is no
    // clock inside wasm32-unknown-unknown.
    tapa_now_ms: () => performance.now(),
  },
};

let engine = null;
let loading = null;

const encoder = new TextEncoder();
const decoder = new TextDecoder();

async function load() {
  // keep the version the page handed us, so the module is cached per release
  const url = new URL("tapa.wasm" + self.location.search, self.location.href);
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`cannot fetch ${url.pathname} (${response.status} ${response.statusText})`);
  }
  // instantiateStreaming needs the right MIME type; fall back if a plain file
  // server sends something else.
  let instance;
  try {
    ({ instance } = await WebAssembly.instantiateStreaming(response, imports));
  } catch {
    const bytes = await response.arrayBuffer();
    ({ instance } = await WebAssembly.instantiate(bytes, imports));
  }
  engine = instance.exports;
}

function call(command) {
  const data = encoder.encode(command);
  const cap = engine.scratch_cap();
  if (data.length > cap) {
    throw new Error(`command is ${data.length} bytes, the buffer holds ${cap}`);
  }
  // the buffer can move when wasm grows its memory, so re-read it every time
  const scratch = new Uint8Array(engine.memory.buffer, engine.scratch_ptr(), data.length);
  scratch.set(data);
  const length = engine.tapa_command(data.length);
  const reply = new Uint8Array(engine.memory.buffer, engine.reply_ptr(), length);
  return JSON.parse(decoder.decode(reply));
}

self.onmessage = async (event) => {
  const { id, cmd } = event.data;
  try {
    if (!engine) {
      loading = loading || load();
      await loading;
    }
    self.postMessage({ id, state: call(cmd) });
  } catch (err) {
    self.postMessage({ id, error: String((err && err.message) || err) });
  }
};
