import type { ReactNode } from "react";
import { AuthContext } from "../src/authApi";

export function TestAccess({ children }: { children: ReactNode }) {
  return (
    <AuthContext.Provider
      value={{
        user: { username: "owner", role: "admin", must_change_password: false },
        expires_at: 9999999999,
      }}
    >
      {children}
    </AuthContext.Provider>
  );
}
