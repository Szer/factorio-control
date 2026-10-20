import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { authKey } from "../src/authApi";
import type { ReactNode } from "react";
import { AuthContext } from "../src/authApi";

const session = {
  user: {
    username: "owner",
    account_id: "fixture-owner",
    role: "admin" as const,
    must_change_password: false,
  },
  expires_at: 9999999999,
};

export function TestAccess({ children }: { children: ReactNode }) {
  const client = useQueryClient();
  useEffect(() => {
    client.setQueryData(authKey, session);
  }, [client]);
  return (
    <AuthContext.Provider value={session}>{children}</AuthContext.Provider>
  );
}
