"use client";

import { useEffect, useRef, useState } from "react";
import { useParams, useRouter } from "next/navigation";
import { getToken } from "@/lib/auth";
import "@xterm/xterm/css/xterm.css";

export default function SshPage() {
  const params = useParams();
  const router = useRouter();
  const envId = params.id as string;
  const termRef = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState<"login" | "connecting" | "connected" | "disconnected">("login");
  const [username, setUsername] = useState("admin");
  const [password, setPassword] = useState("");
  const [creds, setCreds] = useState<{ username: string; password: string } | null>(null);

  const wsRef = useRef<WebSocket | null>(null);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const termInstanceRef = useRef<any>(null);

  // Connect after creds are set and terminal div is mounted
  useEffect(() => {
    if (!creds || !termRef.current) return;

    const token = getToken();
    if (!token) {
      router.push("/login");
      return;
    }

    let ws: WebSocket | null = null;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    let term: any = null;

    async function init() {
      const { Terminal } = await import("@xterm/xterm");
      const { FitAddon } = await import("@xterm/addon-fit");
      if (!termRef.current) return;

      const fitAddon = new FitAddon();
      term = new Terminal({
        cursorBlink: true,
        fontSize: 14,
        fontFamily: "'SF Mono', 'Menlo', 'Courier New', monospace",
        theme: {
          background: "#0a0a0a",
          foreground: "#e4e4e7",
          cursor: "#e4e4e7",
          selectionBackground: "#3f3f46",
        },
      });
      term.loadAddon(fitAddon);
      term.open(termRef.current);
      fitAddon.fit();
      termInstanceRef.current = term;

      term.writeln("Connecting...");

      const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
      const apiHost = process.env.NEXT_PUBLIC_API_URL
        ? new URL(process.env.NEXT_PUBLIC_API_URL).host
        : window.location.host;

      const wsUrl = `${protocol}//${apiHost}/v1/environments/${envId}/ws/ssh?token=${encodeURIComponent(token!)}`;

      ws = new WebSocket(wsUrl);
      ws.binaryType = "arraybuffer";
      wsRef.current = ws;

      ws.onopen = () => {
        ws!.send(JSON.stringify({
          username: creds!.username,
          password: creds!.password,
          cols: term.cols,
          rows: term.rows,
        }));
      };

      ws.onmessage = (event) => {
        setStatus("connected");
        if (event.data instanceof ArrayBuffer) {
          term.write(new Uint8Array(event.data));
        } else {
          term.write(event.data);
        }
      };

      ws.onclose = () => {
        setStatus("disconnected");
        term?.writeln("\r\n\r\n[Connection closed]");
      };

      ws.onerror = () => {
        setStatus("disconnected");
        term?.writeln("\r\n[Connection error]");
      };

      term.onData((data: string) => {
        if (ws && ws.readyState === WebSocket.OPEN) {
          ws.send(data);
        }
      });

      const handleResize = () => fitAddon.fit();
      window.addEventListener("resize", handleResize);

      return () => window.removeEventListener("resize", handleResize);
    }

    init();

    return () => {
      ws?.close();
      term?.dispose();
    };
  }, [creds, envId, router]);

  function handleConnect() {
    setStatus("connecting");
    setCreds({ username, password });
  }

  return (
    <div className="flex flex-col h-[calc(100vh-4rem)]">
      <div className="flex items-center justify-between px-4 py-2 border-b border-zinc-800">
        <div className="flex items-center gap-3">
          <button
            onClick={() => router.push(`/environments/${envId}`)}
            className="text-sm text-zinc-400 hover:text-zinc-200"
          >
            &larr; Back
          </button>
          <span className="text-sm text-zinc-300">
            SSH &mdash; {envId.slice(0, 8)}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <span
            className={`inline-block w-2 h-2 rounded-full ${
              status === "connected"
                ? "bg-green-500"
                : status === "connecting"
                  ? "bg-yellow-500 animate-pulse"
                  : status === "login"
                    ? "bg-zinc-500"
                    : "bg-red-500"
            }`}
          />
          <span className="text-xs text-zinc-500 capitalize">{status}</span>
        </div>
      </div>
      {status === "login" ? (
        <div className="flex-1 flex items-center justify-center bg-[#0a0a0a]">
          <form
            onSubmit={(e) => {
              e.preventDefault();
              handleConnect();
            }}
            className="w-full max-w-sm space-y-4 p-6"
          >
            <div>
              <label className="block text-xs text-zinc-500 mb-1">Username</label>
              <input
                type="text"
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                autoFocus
                className="w-full rounded-lg border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-200 focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
              />
            </div>
            <div>
              <label className="block text-xs text-zinc-500 mb-1">Password</label>
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                className="w-full rounded-lg border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-200 focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
              />
            </div>
            <button
              type="submit"
              className="w-full rounded-lg bg-green-700 px-4 py-2.5 text-sm font-medium text-white hover:bg-green-600 transition-colors"
            >
              Connect
            </button>
          </form>
        </div>
      ) : (
        <div ref={termRef} className="flex-1 bg-[#0a0a0a] p-1" />
      )}
    </div>
  );
}
