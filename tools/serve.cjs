// A tiny static file server for the web front end, with no dependencies.
//
//   node tools/serve.cjs [port] [root]
//
// WebAssembly has to be fetched over HTTP(S): opening index.html as a file://
// URL will not load web/tapa.wasm in any current browser. This serves the
// web/ directory with the MIME types the streaming instantiation path wants.
// Point `root` at a directory to test a different layout, e.g. the subdirectory
// GitHub Pages serves project sites from.

const http = require("node:http");
const fs = require("node:fs");
const path = require("node:path");

const port = Number(process.argv[2] || 8080);
const root = path.resolve(process.argv[3] || path.join(__dirname, "..", "web"));

const types = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".wasm": "application/wasm",
  ".json": "application/json; charset=utf-8",
  ".png": "image/png",
  ".svg": "image/svg+xml",
};

const server = http.createServer((request, response) => {
  const url = new URL(request.url, `http://${request.headers.host || "127.0.0.1"}`);
  let relative = decodeURIComponent(url.pathname);
  if (relative.endsWith("/")) {
    relative += "index.html";
  }
  const file = path.join(root, path.normalize(relative).replace(/^([/\\])+/, ""));

  if (!file.startsWith(root)) {
    response.writeHead(403).end("forbidden");
    return;
  }
  fs.readFile(file, (err, data) => {
    if (err) {
      response.writeHead(404, { "content-type": "text/plain" }).end("not found");
      return;
    }
    response.writeHead(200, {
      "content-type": types[path.extname(file).toLowerCase()] || "application/octet-stream",
      "cache-control": "no-store",
    });
    response.end(data);
  });
});

server.listen(port, "127.0.0.1", () => {
  console.log(`serving ${root}`);
  console.log(`open http://127.0.0.1:${port}/`);
});
