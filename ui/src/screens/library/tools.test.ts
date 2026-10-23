import { describe, expect, it } from "vitest";

import { sample } from "@/test/fixtures";

import { diffTools, isReadOnly, parseTools } from "./tools";
import { looksSecret, movesToVault, parseLines, variablesIn } from "./text";

const tool = (name: string, extra: Record<string, unknown> = {}) => ({
  name,
  description: `${name} tool`,
  inputSchema: { type: "object" },
  ...extra,
});

describe("a tool list", () => {
  it("is read from the JSON the core keeps, and is empty when it is not a list of tools", () => {
    expect(parseTools([tool("a"), tool("b")]).map((t) => t.name)).toEqual(["a", "b"]);
    expect(parseTools(null)).toEqual([]);
    expect(parseTools({ tools: [] })).toEqual([]);
    expect(parseTools([{ nope: 1 }])).toEqual([]);
    const seen = sample.library[0]!.catalog!.seen_tools;
    expect(parseTools(seen)[0]).toMatchObject({ name: "search_issues" });
  });

  it("calls a tool read-only only when the server says so with a true hint", () => {
    expect(isReadOnly(tool("a", { annotations: { readOnlyHint: true } }))).toBe(true);
    expect(isReadOnly(tool("a", { annotations: { readOnlyHint: "yes" } }))).toBe(false);
    expect(isReadOnly(tool("a", { annotations: null }))).toBe(false);
    expect(isReadOnly(tool("a"))).toBe(false);
  });

  it("tells what was added, removed and changed, and which part of a changed tool", () => {
    const accepted = parseTools([
      tool("kept"),
      tool("gone"),
      tool("words"),
      tool("shape", { inputSchema: { type: "object", properties: { a: {} } } }),
      tool("hint", { annotations: { readOnlyHint: true } }),
    ]);
    const seen = parseTools([
      tool("kept", { title: "only the title differs" }),
      tool("words", { description: "other words" }),
      // The same schema with its keys in another order is the same schema.
      tool("shape", { inputSchema: { properties: { a: {} }, type: "object" } }),
      tool("hint", { annotations: { readOnlyHint: false } }),
      tool("new"),
    ]);
    const diff = diffTools(accepted, seen);
    expect(diff.added.map((t) => t.name)).toEqual(["new"]);
    expect(diff.removed.map((t) => t.name)).toEqual(["gone"]);
    expect(diff.changed.map((c) => [c.tool.name, c.parts])).toEqual([
      ["words", ["description"]],
      ["hint", ["annotations"]],
    ]);
    const inputs = diffTools(
      parseTools([tool("t")]),
      parseTools([tool("t", { inputSchema: { type: "string" } })]),
    );
    expect(inputs.changed.map((c) => c.parts)).toEqual([["inputs"]]);
  });
});

describe("the text of the server form", () => {
  it("reads one KEY=VALUE or Name: value per line, and says which line is wrong", () => {
    expect(parseLines("A=1\n\n B = x=y \n", "=")).toEqual({
      ok: true,
      entries: { A: "1", B: "x=y" },
    });
    expect(parseLines("Authorization: Bearer ${T}", ":")).toEqual({
      ok: true,
      entries: { Authorization: "Bearer ${T}" },
    });
    expect(parseLines("fine=1\nbroken", "=")).toEqual({ ok: false, line: 2 });
    expect(parseLines("=empty", "=")).toEqual({ ok: false, line: 1 });
  });

  it("finds the variables a config reads, once each", () => {
    expect(variablesIn(["${A}/x", "no", "${B}${A}", "$C", "${1X}"])).toEqual(["A", "B"]);
  });

  it("flags a key that looks like a secret, by its whole words", () => {
    // The same table as the core's `looks_secret` test.
    const secret = [
      "API_TOKEN",
      "secret",
      "PASSWORD",
      "Authorization",
      "SF_TOKEN",
      "api_key",
      "API_KEY",
      "GITHUB_TOKEN",
      "CLIENT_SECRET",
      "DB_PASSWORD",
      "Password",
      "X-Auth",
      "x-api-key",
      "MYSQL_PWD",
      "LDAP_PASSWD",
      "AWS.CREDENTIALS",
      "GOOGLE_APPLICATION_CREDENTIAL",
      "OPENAI_APIKEY",
      "accessToken",
      "clientSecret",
      "authToken",
      "OAuthToken",
      "MyKey",
      "apiKey",
      "APIKey",
    ];
    for (const key of secret) expect(looksSecret(key), key).toBe(true);
    const plain = [
      "MODE",
      "PATH",
      "Accept",
      "region",
      "TOKEN_URL",
      "AUTH_URL",
      "OAUTH_ISSUER",
      "KEYCLOAK_REALM",
      "API_KEY_HEADER",
      "AUTHOR",
      "KEY_ID",
      "SECRET_NAME",
      "TOKEN_ENDPOINT",
      "AUTH_MODE",
      "PSModulePath",
      "keyName",
      "authorName",
      "tokenUrl",
      "clientId",
    ];
    for (const key of plain) expect(looksSecret(key), key).toBe(false);
  });

  it("moves a secret-looking value unless it is a reference, a plain URL or a path", () => {
    expect(movesToVault("API_KEY", "abc-123-value")).toBe(true);
    expect(movesToVault("API_KEY", "${MY_KEY}")).toBe(false);
    expect(movesToVault("API_KEY", "  ")).toBe(false);
    expect(movesToVault("MODE", "abc-123-value")).toBe(false);
    expect(movesToVault("AUTH", "https://login.example.com/realms/acme")).toBe(false);
    expect(movesToVault("GOOGLE_APPLICATION_CREDENTIALS", String.raw`C:\keys\sa.json`)).toBe(false);
    expect(movesToVault("SSH_KEY", "~/.ssh/id_ed25519")).toBe(false);
    expect(movesToVault("AWS_CREDENTIALS", "/home/me/.aws/credentials")).toBe(false);
    expect(movesToVault("DB_PASSWORD", "https://admin:pw@db.example.com")).toBe(true);
    expect(movesToVault("API_KEY", "/aB3dE+fG7hI/jK9lM=")).toBe(true);
    const moves = [
      ["SLACK_TOKEN", "https://hooks.slack.com/services/T0/B0/abc123"],
      ["WEBHOOK_SECRET", "https://example.invalid/hook/abc123"],
      ["API_KEY", "https://example.invalid/v1"],
      ["AUTH_SERVER", "https://example.invalid/login?token=abc123"],
      ["DB_PASSWORD", "https://user:pw@db.invalid"],
    ] as const;
    for (const [key, value] of moves) expect(movesToVault(key, value), key).toBe(true);
    const stays = [
      ["AUTH_SERVER", "https://auth.example.invalid/realms/x"],
      ["GOOGLE_APPLICATION_CREDENTIALS", String.raw`C:\keys\sa.json`],
      ["GITHUB_TOKEN", "./token.txt"],
    ] as const;
    for (const [key, value] of stays) expect(movesToVault(key, value), key).toBe(false);
  });
});
