"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { setToken } from "@/lib/auth";

export default function LoginPage() {
  const router = useRouter();
  const [token, setTokenValue] = useState("");
  const [error, setError] = useState("");

  function handleLogin() {
    const trimmed = token.trim();
    if (!trimmed) {
      setError("Please enter a JWT token.");
      return;
    }
    setToken(trimmed);
    router.push("/dashboard");
  }

  return (
    <div className="min-h-screen flex items-center justify-center px-4">
      <div className="w-full max-w-md space-y-6">
        <div className="text-center">
          <h1 className="text-3xl font-bold tracking-tight">Orion Complex</h1>
          <p className="mt-2 text-gray-400">Sign in to manage your VMs</p>
        </div>

        <div className="bg-gray-900 border border-gray-800 rounded-xl p-6 space-y-4">
          <div>
            <label
              htmlFor="token"
              className="block text-sm font-medium text-gray-300 mb-1"
            >
              JWT Token
            </label>
            <input
              id="token"
              type="text"
              value={token}
              onChange={(e) => {
                setTokenValue(e.target.value);
                setError("");
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") handleLogin();
              }}
              placeholder="Paste your JWT token"
              className="w-full rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-100 placeholder-gray-500 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
            />
            {error && (
              <p className="mt-1 text-sm text-red-400">{error}</p>
            )}
          </div>

          <button
            onClick={handleLogin}
            className="w-full rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-500 transition-colors"
          >
            Login
          </button>

          <div className="relative my-4">
            <div className="absolute inset-0 flex items-center">
              <div className="w-full border-t border-gray-700" />
            </div>
            <div className="relative flex justify-center text-xs">
              <span className="bg-gray-900 px-2 text-gray-500">
                or continue with
              </span>
            </div>
          </div>

          <div className="space-y-2">
            <div className="relative group">
              <button
                disabled
                className="w-full rounded-lg border border-gray-700 bg-gray-800 px-4 py-2 text-sm font-medium text-gray-500 cursor-not-allowed"
              >
                Sign in with Google
              </button>
              <span className="absolute -top-8 left-1/2 -translate-x-1/2 rounded bg-gray-700 px-2 py-1 text-xs text-gray-300 opacity-0 group-hover:opacity-100 transition-opacity whitespace-nowrap pointer-events-none">
                Coming soon
              </span>
            </div>

            <div className="relative group">
              <button
                disabled
                className="w-full rounded-lg border border-gray-700 bg-gray-800 px-4 py-2 text-sm font-medium text-gray-500 cursor-not-allowed"
              >
                Sign in with Microsoft
              </button>
              <span className="absolute -top-8 left-1/2 -translate-x-1/2 rounded bg-gray-700 px-2 py-1 text-xs text-gray-300 opacity-0 group-hover:opacity-100 transition-opacity whitespace-nowrap pointer-events-none">
                Coming soon
              </span>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
