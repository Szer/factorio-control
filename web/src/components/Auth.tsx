import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  AuthContext,
  authApi,
  authEvent,
  authExpired,
  authKey,
  roleSchema,
  useAccess,
} from "../authApi";
import type {
  Account,
  AccountsDocument,
  Role,
  SessionStatus,
} from "../authApi";
import "../auth.css";

function message(error: unknown) {
  return error instanceof Error ? error.message : "Account request failed";
}

function Login() {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const client = useQueryClient();
  const login = useMutation({
    mutationFn: () => authApi.login(username, password),
    onSuccess: async (session) => {
      setPassword("");
      await client.cancelQueries();
      client.removeQueries({
        predicate: (query) => query.queryKey[0] !== "auth",
      });
      client.getMutationCache().clear();
      client.setQueryData(authKey, session);
    },
    onError: () => {
      setPassword("");
    },
  });
  return (
    <main className="auth-login">
      <section className="panel">
        <header className="panel-header">
          <h1>Factorio Control</h1>
        </header>
        <form
          className="auth-form"
          onSubmit={(event) => {
            event.preventDefault();
            login.mutate();
          }}
        >
          <h2>Sign in</h2>
          <label>
            Account name
            <input
              autoComplete="username"
              autoFocus
              maxLength={64}
              required
              value={username}
              onChange={(event) => {
                setUsername(event.target.value);
              }}
            />
          </label>
          <label>
            Password
            <input
              type="password"
              autoComplete="current-password"
              maxLength={1024}
              required
              value={password}
              onChange={(event) => {
                setPassword(event.target.value);
              }}
            />
          </label>
          {login.isError && (
            <p className="notice error" role="alert">
              {message(login.error)}
            </p>
          )}
          <button
            className="button primary"
            disabled={login.isPending}
            type="submit"
          >
            {login.isPending ? "Signing in…" : "Sign in"}
          </button>
        </form>
      </section>
    </main>
  );
}

export function AuthGate({ children }: { children: ReactNode }) {
  const client = useQueryClient();
  const session = useQuery({
    queryKey: authKey,
    queryFn: authApi.status,
    retry: false,
    staleTime: 30_000,
    refetchInterval: 60_000,
  });
  const identity = session.data
    ? `${session.data.user?.username ?? "signed-out"}:${session.data.user?.role ?? "none"}:${String(session.data.user?.must_change_password ?? false)}`
    : null;
  const [readyIdentity, setReadyIdentity] = useState<string | null>(null);
  useEffect(() => {
    if (identity === null) return;
    let mounted = true;
    client
      .cancelQueries({ predicate: (query) => query.queryKey[0] !== "auth" })
      .then(() => {
        if (!mounted) return;
        client.removeQueries({
          predicate: (query) => query.queryKey[0] !== "auth",
        });
        client.getMutationCache().clear();
        setReadyIdentity(identity);
      })
      .catch(() => {
        if (mounted)
          client.setQueryData<SessionStatus>(authKey, {
            user: null,
            expires_at: null,
          });
      });
    return () => {
      mounted = false;
    };
  }, [client, identity]);
  useEffect(() => {
    let mounted = true;
    const expire = () => {
      client
        .cancelQueries()
        .then(() => {
          if (!mounted) return;
          client.removeQueries({
            predicate: (query) => query.queryKey[0] !== "auth",
          });
          client.getMutationCache().clear();
          client.setQueryData<SessionStatus>(authKey, {
            user: null,
            expires_at: null,
          });
        })
        .catch(() => {
          if (mounted)
            client.setQueryData<SessionStatus>(authKey, {
              user: null,
              expires_at: null,
            });
        });
    };
    window.addEventListener(authEvent, expire);
    return () => {
      mounted = false;
      window.removeEventListener(authEvent, expire);
    };
  }, [client]);
  if (session.isPending)
    return (
      <main className="auth-login" role="status">
        Connecting…
      </main>
    );
  if (session.isError)
    return (
      <main className="auth-login">
        <section className="panel auth-form">
          <p role="alert">{message(session.error)}</p>
          <button
            className="button"
            onClick={() => {
              session.refetch().catch(() => undefined);
            }}
          >
            Retry connection
          </button>
        </section>
      </main>
    );
  if (!session.data.user) return <Login />;
  if (session.data.user.must_change_password)
    return (
      <main className="auth-login">
        <section className="panel">
          <header className="panel-header">
            <h1>Change temporary password</h1>
          </header>
          <p className="auth-form">
            Set your own password to unlock the panel. The bootstrap credential
            file will be deleted.
          </p>
          <PasswordForm
            changed={() => {
              authExpired();
              return Promise.resolve();
            }}
          />
          <button
            className="button"
            onClick={() => {
              authApi.logout().then(authExpired).catch(authExpired);
            }}
          >
            Sign out
          </button>
        </section>
      </main>
    );
  if (readyIdentity !== identity)
    return (
      <main className="auth-login" role="status">
        Connecting…
      </main>
    );
  return (
    <AuthContext.Provider key={identity} value={session.data}>
      {children}
    </AuthContext.Provider>
  );
}

function PasswordForm({ changed }: { changed: () => Promise<void> }) {
  const [current, setCurrent] = useState("");
  const [password, setPassword] = useState("");
  const change = useMutation({
    mutationFn: () => authApi.password(current, password),
    onSuccess: async () => {
      setCurrent("");
      setPassword("");
      await changed();
    },
    onError: () => {
      setCurrent("");
      setPassword("");
    },
  });
  return (
    <form
      className="auth-form auth-password"
      onSubmit={(event) => {
        event.preventDefault();
        change.mutate();
      }}
    >
      <h3>Change password</h3>
      <label>
        Current password
        <input
          type="password"
          autoComplete="current-password"
          maxLength={1024}
          required
          value={current}
          onChange={(event) => {
            setCurrent(event.target.value);
          }}
        />
      </label>
      <label>
        New password
        <input
          type="password"
          autoComplete="new-password"
          minLength={12}
          maxLength={1024}
          required
          value={password}
          onChange={(event) => {
            setPassword(event.target.value);
          }}
        />
      </label>
      <p className="muted">
        At least 12 characters. All your sessions will be signed out.
      </p>
      {change.isError && (
        <p className="notice error" role="alert">
          {message(change.error)}
        </p>
      )}
      <button className="button" disabled={change.isPending} type="submit">
        {change.isPending ? "Saving…" : "Change password"}
      </button>
    </form>
  );
}

export function SessionMenu() {
  const { user } = useAccess();
  const client = useQueryClient();
  const reset = async () => {
    await client.cancelQueries();
    client.removeQueries({
      predicate: (query) => query.queryKey[0] !== "auth",
    });
    client.getMutationCache().clear();
    client.setQueryData<SessionStatus>(authKey, {
      user: null,
      expires_at: null,
    });
  };
  const logout = useMutation({ mutationFn: authApi.logout, onSuccess: reset });
  if (!user) return null;
  return (
    <details className="auth-menu">
      <summary>
        {user.username} <span className="muted">{user.role}</span>
      </summary>
      <div className="panel auth-menu-panel">
        <PasswordForm changed={reset} />
        <button
          className="button"
          disabled={logout.isPending}
          onClick={() => {
            logout.mutate();
          }}
        >
          Sign out
        </button>
        {logout.isError && <p role="alert">{message(logout.error)}</p>}
      </div>
    </details>
  );
}

function RoleSelect({
  value,
  change,
}: {
  value: Role;
  change: (role: Role) => void;
}) {
  return (
    <select
      aria-label="Account role"
      value={value}
      onChange={(event) => {
        change(roleSchema.parse(event.target.value));
      }}
    >
      <option value="viewer">Viewer</option>
      <option value="operator">Operator</option>
      <option value="admin">Administrator</option>
    </select>
  );
}

function AccountEditor({
  account,
  document,
  close,
}: {
  account: Account;
  document: AccountsDocument;
  close: () => void;
}) {
  const { user } = useAccess();
  const client = useQueryClient();
  const [role, setRole] = useState(account.role);
  const [password, setPassword] = useState("");
  const [confirmDelete, setConfirmDelete] = useState(false);
  const save = useMutation({
    mutationFn: () =>
      authApi.update({
        revision: document.revision,
        username: account.username,
        role,
        password: password || null,
      }),
    onSuccess: async (result) => {
      setPassword("");
      client.setQueryData(["accounts"], result);
      close();
      await client.invalidateQueries({ queryKey: authKey });
    },
  });
  const remove = useMutation({
    mutationFn: () => authApi.delete(document.revision, account.username),
    onSuccess: (result) => {
      client.setQueryData(["accounts"], result);
      close();
    },
  });
  return (
    <form
      className="auth-form auth-account-edit"
      onSubmit={(event) => {
        event.preventDefault();
        save.mutate();
      }}
    >
      <h3>Edit {account.username}</h3>
      <label>
        Role
        <RoleSelect value={role} change={setRole} />
      </label>
      <label>
        New password (optional)
        <input
          type="password"
          autoComplete="new-password"
          minLength={12}
          maxLength={1024}
          value={password}
          onChange={(event) => {
            setPassword(event.target.value);
          }}
        />
      </label>
      <p className="muted">Saving signs this account out of all sessions.</p>
      {(save.isError || remove.isError) && (
        <p className="notice error" role="alert">
          {message(save.error ?? remove.error)}
        </p>
      )}
      <div className="auth-actions">
        <button
          className="button primary"
          disabled={save.isPending || remove.isPending}
          type="submit"
        >
          Save account
        </button>
        <button className="button" type="button" onClick={close}>
          Cancel
        </button>
      </div>
      {user?.username !== account.username && (
        <div className="auth-delete">
          <label>
            <input
              type="checkbox"
              checked={confirmDelete}
              onChange={(event) => {
                setConfirmDelete(event.target.checked);
              }}
            />{" "}
            Delete {account.username}
          </label>
          <button
            className="button danger"
            type="button"
            disabled={!confirmDelete || save.isPending || remove.isPending}
            onClick={() => {
              remove.mutate();
            }}
          >
            Delete account
          </button>
        </div>
      )}
    </form>
  );
}

export function AccountPanel() {
  const { canAdmin } = useAccess();
  const client = useQueryClient();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [role, setRole] = useState<Role>("viewer");
  const [editing, setEditing] = useState<Account | null>(null);
  const query = useQuery({
    queryKey: ["accounts"],
    queryFn: authApi.accounts,
    enabled: canAdmin,
    retry: false,
  });
  const create = useMutation({
    mutationFn: () => {
      if (!query.data) throw new Error("Reload accounts before saving");
      return authApi.create({
        revision: query.data.revision,
        username,
        password,
        role,
      });
    },
    onSuccess: (result) => {
      client.setQueryData(["accounts"], result);
      setUsername("");
      setPassword("");
      setRole("viewer");
    },
  });
  if (!canAdmin) return null;
  return (
    <section className="panel auth-account-panel">
      <header className="panel-header">
        <h2>Panel accounts</h2>
      </header>
      <div className="auth-accounts">
        <p className="muted">
          Viewers can inspect the server. Operators can manage the game, saves
          and mods. Administrators also manage settings and accounts.
        </p>
        {query.isError && (
          <p className="notice error" role="alert">
            {message(query.error)}{" "}
            <button
              className="button"
              onClick={() => {
                query.refetch().catch(() => undefined);
              }}
            >
              Reload accounts
            </button>
          </p>
        )}
        {query.isPending && <p role="status">Loading accounts…</p>}
        {query.data && (
          <>
            <div className="table-scroll">
              <table>
                <thead>
                  <tr>
                    <th>Account</th>
                    <th>Role</th>
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {query.data.accounts.map((account) => (
                    <tr key={account.username}>
                      <td>{account.username}</td>
                      <td>{account.role}</td>
                      <td>
                        <button
                          className="button"
                          onClick={() => {
                            setEditing(account);
                          }}
                        >
                          Edit
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            {editing && (
              <AccountEditor
                key={editing.username}
                account={editing}
                document={query.data}
                close={() => {
                  setEditing(null);
                }}
              />
            )}
            <form
              className="auth-form auth-create"
              onSubmit={(event) => {
                event.preventDefault();
                create.mutate();
              }}
            >
              <h3>Create account</h3>
              <label>
                Account name
                <input
                  autoComplete="off"
                  pattern="[A-Za-z0-9_.-]+"
                  maxLength={64}
                  required
                  value={username}
                  onChange={(event) => {
                    setUsername(event.target.value);
                  }}
                />
              </label>
              <label>
                Password
                <input
                  type="password"
                  autoComplete="new-password"
                  minLength={12}
                  maxLength={1024}
                  required
                  value={password}
                  onChange={(event) => {
                    setPassword(event.target.value);
                  }}
                />
              </label>
              <label>
                Role
                <RoleSelect value={role} change={setRole} />
              </label>
              {create.isError && (
                <p className="notice error" role="alert">
                  {message(create.error)}
                </p>
              )}
              <button
                className="button primary"
                disabled={create.isPending}
                type="submit"
              >
                Create account
              </button>
            </form>
          </>
        )}
      </div>
    </section>
  );
}
