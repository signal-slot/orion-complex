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
  const [status, setStatus] = useState<"connecting" | "connected" | "disconnected">("connecting");

  useEffect(() => {
    let ws: WebSocket | null = null;
    let term: import("@xterm/xterm").Terminal | null = null;

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

      const token = getToken();
      if (!token) {
        term.writeln("\r\nNot authenticated. Redirecting...");
        router.push("/login");
        return;
      }

      const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
      const apiHost = process.env.NEXT_PUBLIC_API_URL
        ? new URL(process.env.NEXT_PUBLIC_API_URL).host
        : window.location.host;

      const wsUrl = `${protocol}//${apiHost}/v1/environments/${envId}/ws/ssh?token=${encodeURIComponent(token)}`;

      term.writeln("Connecting...");

      ws = new WebSocket(wsUrl);
      ws.binaryType = "arraybuffer";

      ws.onopen = () => {
        setStatus("connected");
      };

      ws.onmessage = (event) => {
        if (event.data instanceof ArrayBuffer) {
          term!.write(new Uint8Array(event.data));
        } else {
          term!.write(event.data);
        }
      };

      ws.onclose = () => {
        setStatus("disconnected");
        term!.writeln("\r\n\r\n[Connection closed]");
      };

      ws.onerror = () => {
        setStatus("disconnected");
        term!.writeln("\r\n[Connection error]");
      };

      term.onData((data) => {
        if (ws && ws.readyState === WebSocket.OPEN) {
          ws.send(data);
        }
      });

      const handleResize = () => {
        fitAddon.fit();
      };
      window.addEventListener("resize", handleResize);

      return () => {
        window.removeEventListener("resize", handleResize);
      };
    }

    init();

    return () => {
      ws?.close();
      term?.dispose();
    };
  }, [envId, router]);

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
                  : "bg-red-500"
            }`}
          />
          <span className="text-xs text-zinc-500 capitalize">{status}</span>
        </div>
      </div>
      <div ref={termRef} className="flex-1 bg-[#0a0a0a] p-1" />
    </div>
  );
}
