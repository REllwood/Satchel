import { AppShell } from "@/components/layout/AppShell";
import { VaultProvider } from "@/features/vault/vault-store";
import { Toaster } from "@/components/ui/sonner";

function App() {
  return (
    <VaultProvider>
      <AppShell />
      <Toaster position="bottom-right" />
    </VaultProvider>
  );
}

export default App;
