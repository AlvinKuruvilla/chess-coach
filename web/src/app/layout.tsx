import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "Chess Coach",
  description: "Why each mistake in my games was a mistake.",
};

export default function RootLayout({ children }: LayoutProps<"/">) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
