// SPDX-License-Identifier: AGPL-3.0-only
// @mvs-feature("offline_storage")
// @mvs-feature:auth_flow
// @mvs-protocol("auth-api-v1")
// @mvs-protocol:token_handshake
export function login(username: string, password: string): Promise<string> {
  return Promise.resolve(`${username}:${password}`);
}

export interface Session {
  token: string;
}

export const buildSession = (token: string): Session => ({ token });
