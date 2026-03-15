"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import type { User } from "@/lib/types";

interface SidebarProps {
  user: User;
  onLogout: () => void;
  open: boolean;
  onClose: () => void;
}

interface NavItem {
  label: string;
  href: string;
}

const mainNav: NavItem[] = [
  { label: "Dashboard", href: "/dashboard" },
  { label: "SSH Keys", href: "/ssh-keys" },
];

const adminNav: NavItem[] = [
  { label: "Nodes", href: "/admin/nodes" },
  { label: "Images", href: "/admin/images" },
];

function NavLink({ item, active, onClick }: { item: NavItem; active: boolean; onClick?: () => void }) {
  return (
    <Link
      href={item.href}
      onClick={onClick}
      className={`block rounded-lg px-3 py-2.5 text-sm font-medium transition-colors ${
        active
          ? "bg-gray-800 text-white"
          : "text-gray-400 hover:bg-gray-800/50 hover:text-gray-200"
      }`}
    >
      {item.label}
    </Link>
  );
}

export default function Sidebar({ user, onLogout, open, onClose }: SidebarProps) {
  const pathname = usePathname();

  const isActive = (href: string) => pathname === href || pathname.startsWith(href + "/");
  const isAdmin = user.role === "admin";

  const sidebarContent = (
    <>
      <div className="px-4 py-5 flex items-center justify-between">
        <h2 className="text-lg font-bold tracking-tight">Orion Complex</h2>
        <button
          onClick={onClose}
          className="lg:hidden p-1 text-gray-400 hover:text-gray-200"
          aria-label="Close menu"
        >
          <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <nav className="flex-1 space-y-1 px-3">
        {mainNav.map((item) => (
          <NavLink key={item.href} item={item} active={isActive(item.href)} onClick={onClose} />
        ))}

        {isAdmin && (
          <>
            <div className="pt-4 pb-1 px-3">
              <p className="text-xs font-semibold uppercase tracking-wider text-gray-500">
                Admin
              </p>
            </div>
            {adminNav.map((item) => (
              <NavLink
                key={item.href}
                item={item}
                active={isActive(item.href)}
                onClick={onClose}
              />
            ))}
          </>
        )}
      </nav>

      <div className="border-t border-gray-800 px-4 py-4">
        <p className="truncate text-sm text-gray-300">
          {user.display_name || user.email || "User"}
        </p>
        <button
          onClick={onLogout}
          className="mt-2 text-sm text-gray-500 hover:text-gray-300 transition-colors"
        >
          Sign out
        </button>
      </div>
    </>
  );

  return (
    <>
      {/* Mobile overlay */}
      {open && (
        <div
          className="fixed inset-0 z-40 bg-black/60 lg:hidden"
          onClick={onClose}
        />
      )}

      {/* Mobile drawer */}
      <aside
        className={`fixed inset-y-0 left-0 z-50 flex w-56 flex-col bg-gray-900 border-r border-gray-800 transition-transform duration-200 lg:hidden ${
          open ? "translate-x-0" : "-translate-x-full"
        }`}
      >
        {sidebarContent}
      </aside>

      {/* Desktop sidebar */}
      <aside className="hidden lg:flex h-screen w-48 flex-col border-r border-gray-800 bg-gray-900 shrink-0">
        {sidebarContent}
      </aside>
    </>
  );
}
