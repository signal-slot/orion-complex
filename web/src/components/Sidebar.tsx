"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import type { User } from "@/lib/types";

interface SidebarProps {
  user: User;
  onLogout: () => void;
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

function NavLink({ item, active }: { item: NavItem; active: boolean }) {
  return (
    <Link
      href={item.href}
      className={`block rounded-lg px-3 py-2 text-sm font-medium transition-colors ${
        active
          ? "bg-gray-800 text-white"
          : "text-gray-400 hover:bg-gray-800/50 hover:text-gray-200"
      }`}
    >
      {item.label}
    </Link>
  );
}

export default function Sidebar({ user, onLogout }: SidebarProps) {
  const pathname = usePathname();

  const isActive = (href: string) => pathname === href || pathname.startsWith(href + "/");
  const isAdmin = user.role === "admin";

  return (
    <aside className="flex h-screen w-60 flex-col border-r border-gray-800 bg-gray-900">
      <div className="px-4 py-5">
        <h2 className="text-lg font-bold tracking-tight">Orion Complex</h2>
      </div>

      <nav className="flex-1 space-y-1 px-3">
        {mainNav.map((item) => (
          <NavLink key={item.href} item={item} active={isActive(item.href)} />
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
              />
            ))}
          </>
        )}
      </nav>

      <div className="border-t border-gray-800 px-4 py-4">
        <p className="truncate text-sm text-gray-300">
          {user.email || user.display_name || "User"}
        </p>
        <button
          onClick={onLogout}
          className="mt-2 text-sm text-gray-500 hover:text-gray-300 transition-colors"
        >
          Sign out
        </button>
      </div>
    </aside>
  );
}
