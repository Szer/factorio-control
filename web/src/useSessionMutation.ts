import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { QueryKey, UseMutationOptions } from "@tanstack/react-query";
import { authKey, captureAuthEpoch } from "./authApi";
import type { SessionStatus } from "./authApi";

export interface SessionMutationContext {
  readonly isCurrent: () => boolean;
  readonly commit: (effect: () => undefined) => boolean;
  readonly publish: (
    key: QueryKey,
    data: unknown,
    effect?: () => undefined,
  ) => Promise<boolean>;
  readonly refresh: (keys: readonly QueryKey[]) => Promise<boolean>;
}

// Publication checks the initiating session after every asynchronous barrier.
export function useSessionMutation<TData, TVariables>(
  options: Omit<
    UseMutationOptions<TData, Error, TVariables, SessionMutationContext>,
    "onMutate"
  >,
) {
  const client = useQueryClient();
  const { onSuccess, onError, onSettled, ...rest } = options;
  const current = (context: SessionMutationContext | undefined) =>
    context?.isCurrent() === true;
  return useMutation<TData, Error, TVariables, SessionMutationContext>({
    ...rest,
    onMutate: () => {
      const epoch = captureAuthEpoch();
      const session = client.getQueryData<SessionStatus>(authKey);
      if (!session?.user || session.user.must_change_password)
        throw new Error("Sign in again before changing server data.");
      const matches = () =>
        captureAuthEpoch() === epoch &&
        client.getQueryData(authKey) === session &&
        (session.expires_at === null || session.expires_at * 1000 > Date.now());
      const commit: SessionMutationContext["commit"] = (effect) => {
        if (!matches()) return false;
        effect();
        return true;
      };
      if (!matches())
        throw new Error("Sign in again before changing server data.");
      const context: SessionMutationContext = {
        isCurrent: matches,
        commit,
        publish: async (key, data, effect) => {
          if (!matches()) return false;
          await client.cancelQueries({ queryKey: key });
          return commit(() => {
            client.setQueryData(key, data);
            effect?.();
          });
        },
        refresh: async (keys) => {
          if (!matches()) return false;
          await Promise.all(
            keys.map((queryKey) => client.invalidateQueries({ queryKey })),
          );
          return matches();
        },
      };
      return context;
    },
    onSuccess: (data, variables, context, mutation) => {
      if (current(context))
        return onSuccess?.(data, variables, context, mutation);
    },
    onError: (error, variables, context, mutation) => {
      if (current(context))
        return onError?.(error, variables, context, mutation);
    },
    onSettled: (data, error, variables, context, mutation) => {
      if (current(context))
        return onSettled?.(data, error, variables, context, mutation);
    },
  });
}
