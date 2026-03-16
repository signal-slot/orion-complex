"use client";

import { useEffect, useRef, useState } from "react";
import { useParams, useRouter } from "next/navigation";
import { getToken } from "@/lib/auth";

export default function VncPage() {
  const params = useParams();
  const router = useRouter();
  const envId = params.id as string;
  const canvasRef = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState<"connecting" | "connected" | "disconnected">("connecting");
  const [errorMsg, setErrorMsg] = useState("");

  useEffect(() => {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    let rfb: any = null;
    let cancelled = false;

    async function init() {
      if (!canvasRef.current) return;

      const token = getToken();
      if (!token) {
        router.push("/login");
        return;
      }

      try {
        const checkRes = await fetch(`/v1/environments/${envId}`, {
          headers: { Authorization: `Bearer ${token}` },
        });
        if (cancelled) return;
        if (!checkRes.ok) {
          const err = await checkRes.json().catch(() => ({ error: "Failed to fetch environment" }));
          throw new Error(err.error || `HTTP ${checkRes.status}`);
        }
        const envData = await checkRes.json();
        if (cancelled) return;
        if (envData.state !== "running") {
          throw new Error(`Machine is ${envData.state}, not running`);
        }
        if (envData.guest_os === "macos") {
          throw new Error("Browser VNC is not supported for macOS VMs (Apple Screen Sharing uses proprietary authentication). Use the native Screen Sharing app instead.");
        }
        if (envData.provider !== "libvirt" && !envData.vnc_host) {
          throw new Error("VNC not available — port forwarding has not been configured for this machine");
        }

        const RFB = await loadNoVNC();
        if (cancelled) return;

        // Clear any leftover DOM from a previous RFB instance (StrictMode)
        while (canvasRef.current.firstChild) {
          canvasRef.current.removeChild(canvasRef.current.firstChild);
        }

        const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
        const apiHost = process.env.NEXT_PUBLIC_API_URL
          ? new URL(process.env.NEXT_PUBLIC_API_URL).host
          : window.location.host;

        const wsUrl = `${protocol}//${apiHost}/v1/environments/${envId}/ws/vnc?token=${encodeURIComponent(token)}`;
        console.log("[VNC] connecting to:", wsUrl.replace(/token=[^&]+/, "token=***"));

        rfb = new RFB(canvasRef.current, wsUrl, {
          wsProtocols: [],
        });

        rfb.scaleViewport = true;
        rfb.resizeSession = true;
        rfb.background = "#0a0a0a";

        rfb.addEventListener("connect", () => {
          console.log("[VNC] connected");
          if (!cancelled) setStatus("connected");
        });

        rfb.addEventListener("disconnect", (e: { detail: { clean: boolean } }) => {
          console.log("[VNC] disconnected, clean:", e.detail.clean);
          if (cancelled) return;
          setStatus("disconnected");
          if (!e.detail.clean) {
            setErrorMsg("VNC connection failed — the VM may still be booting. Wait a moment and reload.");
          }
        });

        rfb.addEventListener("securityfailure", (e: { detail: { reason: string } }) => {
          console.log("[VNC] security failure:", e.detail.reason);
          if (cancelled) return;
          setStatus("disconnected");
          setErrorMsg(`VNC security error: ${e.detail.reason}`);
        });
      } catch (err) {
        console.error("[VNC] error:", err);
        if (cancelled) return;
        setStatus("disconnected");
        setErrorMsg(err instanceof Error ? err.message : String(err));
      }
    }

    init();

    return () => {
      cancelled = true;
      if (rfb) {
        try { rfb.disconnect(); } catch { /* ignore */ }
      }
    };
  }, [envId, router]);

  return (
    <div className="fixed inset-0 z-50 flex flex-col bg-[#0a0a0a]">
      <div className="flex items-center justify-between px-4 py-2 border-b border-gray-800 bg-gray-900 shrink-0">
        <div className="flex items-center gap-3">
          <button
            onClick={() => router.push(`/environments/${envId}`)}
            className="text-sm text-gray-400 hover:text-gray-200"
          >
            &larr; Back
          </button>
          <span className="text-sm text-gray-300">
            VNC &mdash; {envId.slice(0, 8)}
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
          <span className="text-xs text-gray-500 capitalize">{status}</span>
        </div>
      </div>
      {errorMsg && (
        <div className="px-4 py-2 bg-red-900/30 border-b border-red-800 text-sm text-red-300">
          {errorMsg}
        </div>
      )}
      <div ref={canvasRef} className="flex-1 min-h-0 touch-none" />
    </div>
  );
}

// Load noVNC from pre-built IIFE bundle in public/novnc-rfb.js
// eslint-disable-next-line @typescript-eslint/no-explicit-any
async function loadNoVNC(): Promise<any> {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  if ((window as any).__noVNC?.default) return (window as any).__noVNC.default;

  const script = document.createElement("script");
  script.src = "/novnc-rfb.js";
  await new Promise<void>((resolve, reject) => {
    script.onload = () => resolve();
    script.onerror = () => reject(new Error("Failed to load noVNC bundle"));
    document.head.appendChild(script);
  });

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const RFB = (window as any).__noVNC?.default;
  if (!RFB) throw new Error("noVNC bundle loaded but RFB class not found");
  return RFB;
}
