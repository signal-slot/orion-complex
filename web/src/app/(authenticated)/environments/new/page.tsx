"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { api } from "@/lib/api";
import type { Image, Node, WinInstallOptions } from "@/lib/types";

const MEMORY_OPTIONS = [
  { label: "4 GB", value: 4 * 1024 * 1024 * 1024 },
  { label: "8 GB", value: 8 * 1024 * 1024 * 1024 },
  { label: "16 GB", value: 16 * 1024 * 1024 * 1024 },
  { label: "32 GB", value: 32 * 1024 * 1024 * 1024 },
];

const DISK_OPTIONS = [
  { label: "20 GB", value: 20 * 1024 * 1024 * 1024 },
  { label: "40 GB", value: 40 * 1024 * 1024 * 1024 },
  { label: "80 GB", value: 80 * 1024 * 1024 * 1024 },
  { label: "160 GB", value: 160 * 1024 * 1024 * 1024 },
];

type Mode = "image" | "iso";

export default function NewEnvironmentPage() {
  const router = useRouter();
  const [images, setImages] = useState<Image[]>([]);
  const [nodes, setNodes] = useState<Node[]>([]);
  const [loading, setLoading] = useState(true);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [mode, setMode] = useState<Mode>("image");
  const [name, setName] = useState("");
  const [imageId, setImageId] = useState("");
  const [isoUrl, setIsoUrl] = useState("");
  const [isoFile, setIsoFile] = useState<File | null>(null);
  const [uploading, setUploading] = useState(false);
  const [uploadPct, setUploadPct] = useState(0);
  const [guestOs, setGuestOs] = useState("windows");
  const [isoProvider, setIsoProvider] = useState("virtualization");
  const [winOpts, setWinOpts] = useState<WinInstallOptions>({
    bypass_tpm: true,
    bypass_secure_boot: true,
    bypass_ram: true,
    bypass_cpu: true,
    language: "en-US",
    timezone: "",
    username: "user",
    password: "",
    auto_login: true,
    auto_partition: false,
    product_key: "",
    skip_oobe: false,
  });
  const [vcpus, setVcpus] = useState(4);
  const [memoryBytes, setMemoryBytes] = useState(MEMORY_OPTIONS[1].value);
  const [diskBytes, setDiskBytes] = useState(DISK_OPTIONS[1].value);
  const [ttlHours, setTtlHours] = useState("");

  useEffect(() => {
    async function fetchData() {
      try {
        const [imgs, nds] = await Promise.all([
          api.listImages(),
          api.listNodes(),
        ]);
        setImages(imgs);
        setNodes(nds);
        if (imgs.length > 0) {
          setImageId(imgs[0].id);
        }
      } catch (err) {
        console.error("Failed to fetch form data:", err);
      } finally {
        setLoading(false);
      }
    }
    fetchData();
  }, []);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setSubmitting(true);
    setError(null);

    try {
      const ttlSeconds = ttlHours ? parseFloat(ttlHours) * 3600 : undefined;

      if (mode === "iso") {
        // Create environment first to check capacity before uploading
        const env = await api.createEnvironment({
          iso_url: isoFile ? "pending-upload" : isoUrl,
          provider: isoProvider,
          guest_os: guestOs,
          guest_arch: "x86_64",
          name: name.trim() || undefined,
          vcpus,
          memory_bytes: memoryBytes,
          disk_bytes: diskBytes,
          ttl_seconds: ttlSeconds,
          win_install_options: guestOs === "windows" ? winOpts : undefined,
        });

        if (isoFile) {
          setUploading(true);
          setUploadPct(0);
          const result = await api.uploadIso(isoFile, setUploadPct);
          await api.updateIsoUrl(env.id, result.path);
          setUploading(false);
        }

        router.push(`/environments/${env.id}`);
        return;
      }

      const env = await api.createEnvironment({
            image_id: imageId,
            name: name.trim() || undefined,
            vcpus,
            memory_bytes: memoryBytes,
            disk_bytes: diskBytes,
            ttl_seconds: ttlSeconds,
          });
      router.push(`/environments/${env.id}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create machine");
    } finally {
      setSubmitting(false);
    }
  }

  const canSubmit = mode === "iso" ? !!(isoUrl || isoFile) : !!imageId;

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="text-zinc-400">Loading...</div>
      </div>
    );
  }

  const inputClass =
    "w-full rounded-lg border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-200 placeholder-zinc-500 focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500";

  return (
    <div className="mx-auto max-w-lg space-y-6">
      <h1 className="text-2xl font-semibold text-zinc-100">New Machine</h1>

      {error && (
        <div className="rounded-lg border border-red-800 bg-red-900/30 px-4 py-3 text-sm text-red-400">
          {error}
        </div>
      )}

      {/* Mode Toggle */}
      <div className={`flex rounded-lg border border-zinc-700 overflow-hidden ${submitting ? "opacity-60 pointer-events-none" : ""}`}>
        <button
          type="button"
          onClick={() => setMode("image")}
          disabled={submitting}
          className={`flex-1 px-4 py-2 text-sm font-medium transition-colors ${
            mode === "image"
              ? "bg-indigo-600 text-white"
              : "bg-zinc-800 text-zinc-400 hover:text-zinc-200"
          }`}
        >
          From Image
        </button>
        <button
          type="button"
          onClick={() => setMode("iso")}
          disabled={submitting}
          className={`flex-1 px-4 py-2 text-sm font-medium transition-colors ${
            mode === "iso"
              ? "bg-indigo-600 text-white"
              : "bg-zinc-800 text-zinc-400 hover:text-zinc-200"
          }`}
        >
          Install from ISO
        </button>
      </div>

      <form onSubmit={handleSubmit} className="space-y-5">
       <fieldset disabled={submitting} className="space-y-5 disabled:opacity-60">
        {/* Name */}
        <div>
          <label htmlFor="name" className="block text-sm font-medium text-zinc-300 mb-1.5">
            Name (optional)
          </label>
          <input
            id="name"
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Auto-generated if empty"
            className={inputClass}
          />
        </div>

        {mode === "image" ? (
          /* Image selector */
          <div>
            <label htmlFor="image" className="block text-sm font-medium text-zinc-300 mb-1.5">
              Image
            </label>
            <select
              id="image"
              value={imageId}
              onChange={(e) => setImageId(e.target.value)}
              className={inputClass}
            >
              {images.map((img) => (
                <option key={img.id} value={img.id}>
                  {img.name ?? img.id.slice(0, 8)} ({img.guest_os}/{img.guest_arch})
                </option>
              ))}
            </select>
          </div>
        ) : (
          /* ISO URL + OS selector */
          <>
            <div>
              <label className="block text-sm font-medium text-zinc-300 mb-1.5">
                ISO
              </label>
              <div className="space-y-2">
                <label className="flex items-center gap-3 rounded-lg border border-zinc-700 bg-zinc-800 px-3 py-2.5 cursor-pointer hover:border-zinc-600 transition-colors">
                  <span className="text-sm text-zinc-400 shrink-0">
                    {isoFile ? isoFile.name : "Choose file..."}
                  </span>
                  <input
                    type="file"
                    accept=".iso,.img"
                    className="hidden"
                    onChange={(e) => {
                      const f = e.target.files?.[0];
                      if (f) { setIsoFile(f); setIsoUrl(""); }
                    }}
                  />
                </label>
                <div className="flex items-center gap-2">
                  <span className="text-xs text-zinc-600">or URL:</span>
                  <input
                    type="text"
                    value={isoUrl}
                    onChange={(e) => { setIsoUrl(e.target.value); setIsoFile(null); }}
                    placeholder="https://..."
                    className={`flex-1 ${inputClass}`}
                  />
                </div>
              </div>
              {uploading && (
                <div className="mt-2">
                  <div className="flex justify-between text-xs text-zinc-400 mb-1">
                    <span>Uploading ISO...</span>
                    <span>{uploadPct}%</span>
                  </div>
                  <div className="h-2 rounded-full bg-zinc-800 overflow-hidden">
                    <div
                      className="h-full rounded-full bg-indigo-500 transition-all duration-300"
                      style={{ width: `${uploadPct}%` }}
                    />
                  </div>
                </div>
              )}
            </div>
            <div className="grid grid-cols-2 gap-3">
              <div>
                <label htmlFor="guest_os" className="block text-sm font-medium text-zinc-300 mb-1.5">
                  OS
                </label>
                <select
                  id="guest_os"
                  value={guestOs}
                  onChange={(e) => setGuestOs(e.target.value)}
                  className={inputClass}
                >
                  <option value="windows">Windows</option>
                  <option value="linux">Linux</option>
                </select>
              </div>
              <div>
                <label htmlFor="iso_provider" className="block text-sm font-medium text-zinc-300 mb-1.5">
                  Provider
                </label>
                <select
                  id="iso_provider"
                  value={isoProvider}
                  onChange={(e) => setIsoProvider(e.target.value)}
                  className={inputClass}
                >
                  <option value="virtualization">macOS (QEMU)</option>
                  <option value="libvirt">Linux (KVM)</option>
                  <option value="hyperv">Windows (Hyper-V)</option>
                </select>
              </div>
            </div>
            {guestOs === "windows" && (
              <div className="space-y-4 rounded-lg border border-zinc-700 bg-zinc-800/50 p-4">
                <h3 className="text-sm font-medium text-zinc-300">Windows Install Options</h3>

                {/* Hardware Bypass */}
                <fieldset className="space-y-1.5">
                  <legend className="text-xs text-zinc-500 mb-1">Hardware Requirement Bypass</legend>
                  {([
                    ["bypass_tpm", "TPM"],
                    ["bypass_secure_boot", "Secure Boot"],
                    ["bypass_ram", "RAM"],
                    ["bypass_cpu", "CPU"],
                  ] as const).map(([key, label]) => (
                    <WinCheckbox key={key} label={label} checked={!!winOpts[key]}
                      onChange={(v) => setWinOpts({ ...winOpts, [key]: v })} />
                  ))}
                </fieldset>

                {/* Language / Timezone */}
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <label className="block text-xs text-zinc-500 mb-1">Language</label>
                    <select value={winOpts.language} onChange={(e) => setWinOpts({ ...winOpts, language: e.target.value })} className={inputClass}>
                      <option value="en-US">English (US)</option>
                      <option value="en-GB">English (UK)</option>
                      <option value="ja-JP">Japanese</option>
                      <option value="zh-CN">Chinese (Simplified)</option>
                      <option value="zh-TW">Chinese (Traditional)</option>
                      <option value="ko-KR">Korean</option>
                      <option value="de-DE">German</option>
                      <option value="fr-FR">French</option>
                      <option value="es-ES">Spanish</option>
                      <option value="pt-BR">Portuguese (Brazil)</option>
                    </select>
                  </div>
                  <div>
                    <label className="block text-xs text-zinc-500 mb-1">Timezone</label>
                    <select value={winOpts.timezone} onChange={(e) => setWinOpts({ ...winOpts, timezone: e.target.value })} className={inputClass}>
                      <option value="">Default</option>
                      <option value="Tokyo Standard Time">Tokyo (JST)</option>
                      <option value="Pacific Standard Time">Pacific (PST)</option>
                      <option value="Eastern Standard Time">Eastern (EST)</option>
                      <option value="Central Standard Time">Central (CST)</option>
                      <option value="Mountain Standard Time">Mountain (MST)</option>
                      <option value="GMT Standard Time">London (GMT)</option>
                      <option value="Central European Standard Time">Central Europe (CET)</option>
                      <option value="China Standard Time">China (CST)</option>
                      <option value="Korea Standard Time">Korea (KST)</option>
                      <option value="UTC">UTC</option>
                    </select>
                  </div>
                </div>

                {/* User Account */}
                <fieldset className="space-y-2">
                  <legend className="text-xs text-zinc-500 mb-1">User Account</legend>
                  <div className="grid grid-cols-2 gap-3">
                    <input type="text" placeholder="Username" value={winOpts.username}
                      onChange={(e) => setWinOpts({ ...winOpts, username: e.target.value })} className={inputClass} />
                    <input type="password" placeholder="Password" value={winOpts.password}
                      onChange={(e) => setWinOpts({ ...winOpts, password: e.target.value })} className={inputClass} />
                  </div>
                  <WinCheckbox label="Auto-login" checked={!!winOpts.auto_login}
                    onChange={(v) => setWinOpts({ ...winOpts, auto_login: v })} />
                </fieldset>

                {/* Partition / Product Key / OOBE */}
                <div className="space-y-1.5">
                  <WinCheckbox label="Auto-partition disk" checked={!!winOpts.auto_partition}
                    onChange={(v) => setWinOpts({ ...winOpts, auto_partition: v })} />
                  <WinCheckbox label="Skip OOBE (initial setup wizard)" checked={!!winOpts.skip_oobe}
                    onChange={(v) => setWinOpts({ ...winOpts, skip_oobe: v })} />
                </div>
                <div>
                  <label className="block text-xs text-zinc-500 mb-1">Product Key (optional)</label>
                  <input type="text" placeholder="XXXXX-XXXXX-XXXXX-XXXXX-XXXXX" value={winOpts.product_key}
                    onChange={(e) => setWinOpts({ ...winOpts, product_key: e.target.value })} className={inputClass} />
                </div>
              </div>
            )}
          </>
        )}

        {/* vCPUs */}
        <div>
          <label htmlFor="vcpus" className="block text-sm font-medium text-zinc-300 mb-1.5">
            vCPUs
          </label>
          <input
            id="vcpus"
            type="number"
            min={1}
            max={64}
            value={vcpus}
            onChange={(e) => setVcpus(parseInt(e.target.value) || 1)}
            className={inputClass}
          />
        </div>

        {/* Memory */}
        <div>
          <label htmlFor="memory" className="block text-sm font-medium text-zinc-300 mb-1.5">
            Memory
          </label>
          <select
            id="memory"
            value={memoryBytes}
            onChange={(e) => setMemoryBytes(Number(e.target.value))}
            className={inputClass}
          >
            {MEMORY_OPTIONS.map((opt) => (
              <option key={opt.value} value={opt.value}>
                {opt.label}
              </option>
            ))}
          </select>
        </div>

        {/* Disk */}
        <div>
          <label htmlFor="disk" className="block text-sm font-medium text-zinc-300 mb-1.5">
            Disk
          </label>
          <select
            id="disk"
            value={diskBytes}
            onChange={(e) => setDiskBytes(Number(e.target.value))}
            className={inputClass}
          >
            {DISK_OPTIONS.map((opt) => (
              <option key={opt.value} value={opt.value}>
                {opt.label}
              </option>
            ))}
          </select>
        </div>

        {/* TTL */}
        <div>
          <label htmlFor="ttl" className="block text-sm font-medium text-zinc-300 mb-1.5">
            TTL (hours, optional)
          </label>
          <input
            id="ttl"
            type="number"
            min={0}
            step={0.5}
            value={ttlHours}
            onChange={(e) => setTtlHours(e.target.value)}
            placeholder="No expiration"
            className={inputClass}
          />
        </div>

        <button
          type="submit"
          disabled={submitting || !canSubmit}
          className="w-full rounded-lg bg-indigo-600 px-4 py-2.5 text-sm font-medium text-white hover:bg-indigo-500 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
        >
          {submitting ? "Creating..." : "Create Machine"}
        </button>
       </fieldset>
      </form>
    </div>
  );
}

function WinCheckbox({ label, checked, onChange }: {
  label: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="flex items-center gap-2 cursor-pointer">
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        className="rounded border-zinc-600 bg-zinc-800 text-indigo-500 focus:ring-indigo-500 focus:ring-offset-0"
      />
      <span className="text-sm text-zinc-300">{label}</span>
    </label>
  );
}
