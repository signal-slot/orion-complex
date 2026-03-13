"use client";

import { useState, useEffect, useRef } from "react";
import { useRouter } from "next/navigation";
import { setToken } from "@/lib/auth";
import {
  webauthnRegisterBegin,
  webauthnRegisterComplete,
  webauthnLoginBegin,
  webauthnLoginComplete,
  totpRegister,
  totpVerify,
  totpLogin,
} from "@/lib/api";
import {
  startRegistration,
  startAuthentication,
} from "@simplewebauthn/browser";
import QRCode from "qrcode";

export default function LoginPage() {
  const router = useRouter();
  const [email, setEmail] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [mode, setMode] = useState<"login" | "register">("login");
  const [webauthnSupported, setWebauthnSupported] = useState(true);

  // TOTP state
  const [totpStep, setTotpStep] = useState<"email" | "qr" | "code">("email");
  const [challengeId, setChallengeId] = useState("");
  const [totpSecret, setTotpSecret] = useState("");
  const [code, setCode] = useState("");
  const qrCanvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const supported =
      typeof window !== "undefined" &&
      window.isSecureContext &&
      !!window.PublicKeyCredential;
    setWebauthnSupported(supported);
  }, []);

  // ── Passkey handlers ──────────────────────────────────────────────

  async function handlePasskeyLogin() {
    const trimmed = email.trim();
    if (!trimmed || !trimmed.includes("@")) {
      setError("Please enter a valid email address.");
      return;
    }

    setLoading(true);
    setError("");

    try {
      const beginResp = await webauthnLoginBegin({ email: trimmed });
      const credential = await startAuthentication({
        optionsJSON: beginResp.publicKey as Parameters<typeof startAuthentication>[0]["optionsJSON"],
      });
      const result = await webauthnLoginComplete({
        challenge_id: beginResp.challenge_id,
        credential,
      });
      setToken(result.token);
      router.push("/dashboard");
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      if (msg.includes("no account found")) {
        setError("No account found. Register first.");
      } else if (msg.includes("ceremony was cancelled") || msg.includes("AbortError") || msg.includes("NotAllowedError")) {
        setError("Authentication cancelled.");
      } else {
        setError(msg);
      }
    } finally {
      setLoading(false);
    }
  }

  async function handlePasskeyRegister() {
    const trimmed = email.trim();
    if (!trimmed || !trimmed.includes("@")) {
      setError("Please enter a valid email address.");
      return;
    }

    setLoading(true);
    setError("");

    try {
      const beginResp = await webauthnRegisterBegin({
        email: trimmed,
        display_name: displayName.trim() || undefined,
      });
      const credential = await startRegistration({
        optionsJSON: beginResp.publicKey as Parameters<typeof startRegistration>[0]["optionsJSON"],
      });
      const result = await webauthnRegisterComplete({
        challenge_id: beginResp.challenge_id,
        credential,
      });
      setToken(result.token);
      router.push("/dashboard");
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      if (msg.includes("ceremony was cancelled") || msg.includes("AbortError") || msg.includes("NotAllowedError")) {
        setError("Registration cancelled.");
      } else {
        setError(msg);
      }
    } finally {
      setLoading(false);
    }
  }

  // ── TOTP handlers ─────────────────────────────────────────────────

  async function handleTotpRegister() {
    const trimmed = email.trim();
    if (!trimmed || !trimmed.includes("@")) {
      setError("Please enter a valid email address.");
      return;
    }

    setLoading(true);
    setError("");

    try {
      const resp = await totpRegister({
        email: trimmed,
        display_name: displayName.trim() || undefined,
      });
      setChallengeId(resp.challenge_id);
      setTotpSecret(resp.secret);
      setTotpStep("qr");

      // Render QR code after state update
      setTimeout(() => {
        if (qrCanvasRef.current) {
          QRCode.toCanvas(qrCanvasRef.current, resp.otpauth_url, {
            width: 200,
            margin: 2,
            color: { dark: "#ffffff", light: "#00000000" },
          });
        }
      }, 50);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(msg);
    } finally {
      setLoading(false);
    }
  }

  async function handleTotpVerify() {
    if (code.length !== 6) {
      setError("Please enter the 6-digit code.");
      return;
    }

    setLoading(true);
    setError("");

    try {
      const result = await totpVerify({ challenge_id: challengeId, code });
      setToken(result.token);
      router.push("/dashboard");
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(msg);
    } finally {
      setLoading(false);
    }
  }

  async function handleTotpLogin() {
    const trimmed = email.trim();
    if (!trimmed || !trimmed.includes("@")) {
      setError("Please enter a valid email address.");
      return;
    }
    if (code.length !== 6) {
      setError("Please enter the 6-digit code.");
      return;
    }

    setLoading(true);
    setError("");

    try {
      const result = await totpLogin({ email: trimmed, code });
      setToken(result.token);
      router.push("/dashboard");
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(msg);
    } finally {
      setLoading(false);
    }
  }

  // ── TOTP UI (fallback) ────────────────────────────────────────────

  if (!webauthnSupported) {
    return (
      <div className="min-h-screen flex items-center justify-center px-4">
        <div className="w-full max-w-md space-y-6">
          <div className="text-center">
            <h1 className="text-3xl font-bold tracking-tight">Orion Complex</h1>
            <p className="mt-2 text-gray-400">Sign in with authenticator app</p>
          </div>

          <div className="bg-gray-900 border border-gray-800 rounded-xl p-6 space-y-4">
            {/* Tab switcher */}
            <div className="flex rounded-lg bg-gray-800 p-1">
              <button
                onClick={() => { setMode("login"); setError(""); setCode(""); setTotpStep("email"); }}
                className={`flex-1 rounded-md px-3 py-1.5 text-sm font-medium transition-colors ${
                  mode === "login" ? "bg-gray-700 text-white" : "text-gray-400 hover:text-gray-300"
                }`}
              >
                Sign In
              </button>
              <button
                onClick={() => { setMode("register"); setError(""); setCode(""); setTotpStep("email"); }}
                className={`flex-1 rounded-md px-3 py-1.5 text-sm font-medium transition-colors ${
                  mode === "register" ? "bg-gray-700 text-white" : "text-gray-400 hover:text-gray-300"
                }`}
              >
                Register
              </button>
            </div>

            {mode === "register" && totpStep === "qr" ? (
              /* QR code step */
              <>
                <p className="text-sm text-gray-300">
                  Scan this QR code with your authenticator app (Google Authenticator, Authy, etc.)
                </p>
                <div className="flex justify-center">
                  <canvas ref={qrCanvasRef} />
                </div>
                <div className="text-center">
                  <p className="text-xs text-gray-500 mb-1">Or enter this key manually:</p>
                  <code className="text-xs text-gray-300 bg-gray-800 px-2 py-1 rounded select-all break-all">
                    {totpSecret}
                  </code>
                </div>
                <div>
                  <label htmlFor="code" className="block text-sm font-medium text-gray-300 mb-1">
                    Verification Code
                  </label>
                  <input
                    id="code"
                    type="text"
                    inputMode="numeric"
                    maxLength={6}
                    value={code}
                    onChange={(e) => { setCode(e.target.value.replace(/\D/g, "")); setError(""); }}
                    onKeyDown={(e) => { if (e.key === "Enter") handleTotpVerify(); }}
                    placeholder="000000"
                    className="w-full rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-center text-2xl font-mono tracking-[0.5em] text-gray-100 placeholder-gray-500 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
                    autoFocus
                  />
                </div>
                {error && <p className="text-sm text-red-400">{error}</p>}
                <button
                  onClick={handleTotpVerify}
                  disabled={loading}
                  className="w-full rounded-lg bg-blue-600 px-4 py-2.5 text-sm font-medium text-white hover:bg-blue-500 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                >
                  {loading ? (
                    <span className="inline-block w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                  ) : (
                    "Verify & Create Account"
                  )}
                </button>
                <button
                  onClick={() => { setTotpStep("email"); setCode(""); setError(""); }}
                  className="w-full text-sm text-gray-400 hover:text-gray-300"
                >
                  Back
                </button>
              </>
            ) : (
              /* Email + code step */
              <>
                <div>
                  <label htmlFor="email" className="block text-sm font-medium text-gray-300 mb-1">
                    Email
                  </label>
                  <input
                    id="email"
                    type="email"
                    value={email}
                    onChange={(e) => { setEmail(e.target.value); setError(""); }}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        mode === "login" ? handleTotpLogin() : handleTotpRegister();
                      }
                    }}
                    placeholder="you@example.com"
                    className="w-full rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-100 placeholder-gray-500 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
                  />
                </div>

                {mode === "register" && (
                  <div>
                    <label htmlFor="displayName" className="block text-sm font-medium text-gray-300 mb-1">
                      Display Name
                      <span className="text-gray-500 ml-1">(optional)</span>
                    </label>
                    <input
                      id="displayName"
                      type="text"
                      value={displayName}
                      onChange={(e) => setDisplayName(e.target.value)}
                      placeholder="Your name"
                      className="w-full rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-100 placeholder-gray-500 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
                    />
                  </div>
                )}

                {mode === "login" && (
                  <div>
                    <label htmlFor="code" className="block text-sm font-medium text-gray-300 mb-1">
                      TOTP Code
                    </label>
                    <input
                      id="code"
                      type="text"
                      inputMode="numeric"
                      maxLength={6}
                      value={code}
                      onChange={(e) => { setCode(e.target.value.replace(/\D/g, "")); setError(""); }}
                      onKeyDown={(e) => { if (e.key === "Enter") handleTotpLogin(); }}
                      placeholder="000000"
                      className="w-full rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-center text-2xl font-mono tracking-[0.5em] text-gray-100 placeholder-gray-500 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
                    />
                  </div>
                )}

                {error && <p className="text-sm text-red-400">{error}</p>}

                <button
                  onClick={mode === "login" ? handleTotpLogin : handleTotpRegister}
                  disabled={loading}
                  className="w-full rounded-lg bg-blue-600 px-4 py-2.5 text-sm font-medium text-white hover:bg-blue-500 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                >
                  {loading ? (
                    <span className="inline-block w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                  ) : mode === "login" ? (
                    "Sign In"
                  ) : (
                    "Set Up Authenticator"
                  )}
                </button>

                <p className="text-center text-xs text-gray-500">
                  {mode === "login"
                    ? "Enter the code from your authenticator app"
                    : "You'll scan a QR code with your authenticator app"}
                </p>
              </>
            )}
          </div>
        </div>
      </div>
    );
  }

  // ── Passkey UI (primary, secure context only) ─────────────────────

  return (
    <div className="min-h-screen flex items-center justify-center px-4">
      <div className="w-full max-w-md space-y-6">
        <div className="text-center">
          <h1 className="text-3xl font-bold tracking-tight">Orion Complex</h1>
          <p className="mt-2 text-gray-400">Sign in to manage your VMs</p>
        </div>

        <div className="bg-gray-900 border border-gray-800 rounded-xl p-6 space-y-4">
          {/* Tab switcher */}
          <div className="flex rounded-lg bg-gray-800 p-1">
            <button
              onClick={() => { setMode("login"); setError(""); }}
              className={`flex-1 rounded-md px-3 py-1.5 text-sm font-medium transition-colors ${
                mode === "login"
                  ? "bg-gray-700 text-white"
                  : "text-gray-400 hover:text-gray-300"
              }`}
            >
              Sign In
            </button>
            <button
              onClick={() => { setMode("register"); setError(""); }}
              className={`flex-1 rounded-md px-3 py-1.5 text-sm font-medium transition-colors ${
                mode === "register"
                  ? "bg-gray-700 text-white"
                  : "text-gray-400 hover:text-gray-300"
              }`}
            >
              Register
            </button>
          </div>

          <div>
            <label htmlFor="email" className="block text-sm font-medium text-gray-300 mb-1">
              Email
            </label>
            <input
              id="email"
              type="email"
              value={email}
              onChange={(e) => { setEmail(e.target.value); setError(""); }}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  mode === "login" ? handlePasskeyLogin() : handlePasskeyRegister();
                }
              }}
              placeholder="you@example.com"
              className="w-full rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-100 placeholder-gray-500 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
            />
          </div>

          {mode === "register" && (
            <div>
              <label htmlFor="displayName" className="block text-sm font-medium text-gray-300 mb-1">
                Display Name
                <span className="text-gray-500 ml-1">(optional)</span>
              </label>
              <input
                id="displayName"
                type="text"
                value={displayName}
                onChange={(e) => setDisplayName(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Enter") handlePasskeyRegister(); }}
                placeholder="Your name"
                className="w-full rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-100 placeholder-gray-500 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
              />
            </div>
          )}

          {error && <p className="text-sm text-red-400">{error}</p>}

          <button
            onClick={mode === "login" ? handlePasskeyLogin : handlePasskeyRegister}
            disabled={loading}
            className="w-full rounded-lg bg-blue-600 px-4 py-2.5 text-sm font-medium text-white hover:bg-blue-500 transition-colors disabled:opacity-50 disabled:cursor-not-allowed flex items-center justify-center gap-2"
          >
            {loading ? (
              <span className="inline-block w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />
            ) : (
              <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M12 11c0 3.517-1.009 6.799-2.753 9.571m-3.44-2.04l.054-.09A13.916 13.916 0 008 11a4 4 0 118 0c0 1.017-.07 2.019-.203 3m-2.118 6.844A21.88 21.88 0 0015.171 17m3.839 1.132c.645-2.266.99-4.659.99-7.132A8 8 0 008 4.07M3 15.364c.64-1.319 1-2.8 1-4.364 0-1.457.39-2.823 1.07-4" />
              </svg>
            )}
            {mode === "login" ? "Sign in with Passkey" : "Register with Passkey"}
          </button>

          <p className="text-center text-xs text-gray-500">
            {mode === "login"
              ? "Use Touch ID, Face ID, or your security key"
              : "Create an account and register a passkey"}
          </p>
        </div>
      </div>
    </div>
  );
}
