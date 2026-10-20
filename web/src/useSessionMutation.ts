import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { UseMutationOptions } from "@tanstack/react-query";
import { authKey } from "./authApi";
import type { SessionStatus } from "./authApi";

// Mutation callbacks belong to the session that sent the request.
export function useSessionMutation<TData, TVariables>(
  options: Omit<
    UseMutationOptions<TData, Error, TVariables, SessionStatus>,
    "onMutate"
  >,
) {
  const client = useQueryClient();
  const { onSuccess, onError, onSettled, ...rest } = options;
  const current = (session: SessionStatus | undefined) =>
    session !== undefined && client.getQueryData(authKey) === session;
  return useMutation<TData, Error, TVariables, SessionStatus>({
    ...rest,
    onMutate: () => {
      const session = client.getQueryData<SessionStatus>(authKey);
      if (!session?.user || session.user.must_change_password)
        throw new Error("Sign in again before changing server data.");
      return session;
    },
    onSuccess: (data, variables, session, context) => {
      if (current(session))
        return onSuccess?.(data, variables, session, context);
    },
    onError: (error, variables, session, context) => {
      if (current(session))
        return onError?.(error, variables, session, context);
    },
    onSettled: (data, error, variables, session, context) => {
      if (current(session))
        return onSettled?.(data, error, variables, session, context);
    },
  });
}
