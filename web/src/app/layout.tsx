import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "Orion Complex",
  description: "Ephemeral macOS VM management",
  viewport: "width=device-width, initial-scale=1, viewport-fit=cover",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body className="bg-gray-950 text-gray-100 font-sans antialiased min-h-screen">
        {children}
      </body>
    </html>
  );
}
