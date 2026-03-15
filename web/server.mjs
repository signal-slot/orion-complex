import { createServer } from "https";
import { readFileSync } from "fs";
import { parse } from "url";
import next from "next";
import httpProxy from "http-proxy";

const dev = process.env.NODE_ENV !== "production";
const hostname = process.env.HOSTNAME || "0.0.0.0";
const port = parseInt(process.env.PORT || "2742", 10);
const apiTarget = process.env.API_URL || "http://127.0.0.1:2743";

const certFile = process.env.TLS_CERT || "../tls-cert.pem";
const keyFile = process.env.TLS_KEY || "../tls-key.pem";

const app = next({ dev, hostname, port, turbopack: false });
const handle = app.getRequestHandler();

const proxy = httpProxy.createProxyServer({
  target: apiTarget,
  ws: true,
  changeOrigin: true,
});

proxy.on("error", (err, _req, res) => {
  console.error("Proxy error:", err.message);
  if (res.writeHead) {
    res.writeHead(502, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ error: "backend unavailable" }));
  }
});

await app.prepare();

const httpsOptions = {
  key: readFileSync(keyFile),
  cert: readFileSync(certFile),
};

const server = createServer(httpsOptions, (req, res) => {
  const parsedUrl = parse(req.url, true);

  // Proxy /v1/* to the API backend
  if (parsedUrl.pathname?.startsWith("/v1/")) {
    proxy.web(req, res);
  } else {
    handle(req, res, parsedUrl);
  }
});

// Handle WebSocket upgrade for /v1/*/ws/*
// Must intercept before Next.js HMR handler gets it
server.on("upgrade", (req, socket, head) => {
  if (req.url?.startsWith("/v1/")) {
    console.log(`> WS upgrade: ${req.url}`);
    proxy.ws(req, socket, head);
  }
});

server.listen(port, hostname, () => {
  console.log(`> Ready on https://${hostname}:${port}`);
  console.log(`> API proxy -> ${apiTarget}`);
});
