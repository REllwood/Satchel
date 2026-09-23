import { Cloud, HardDrive, type LucideProps } from "lucide-react";

import type { Provider } from "@/lib/api";

/** Icon for where a vault lives: a cloud for synced folders, a disk for local. */
export function ProviderIcon({ provider, ...props }: { provider: Provider } & LucideProps) {
  return provider === "local" ? <HardDrive {...props} /> : <Cloud {...props} />;
}
