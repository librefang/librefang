#!/usr/bin/env node
/**
 * Example sidecar channel adapter for LibreFang (Node.js)
 *
 * Usage in config.toml:
 *   [[sidecar_channels]]
 *   name = "node-echo"
 *   command = "node"
 *   args = ["examples/sidecar-channel-node/adapter.js"]
 */

const readline = require("readline");

function sendEvent(method, params) {
  const event = { method };
  if (params) event.params = params;
  process.stdout.write(JSON.stringify(event) + "\n");
}

function handleCommand(cmd) {
  if (!cmd || typeof cmd !== "object" || Array.isArray(cmd) || typeof cmd.method !== "string") {
    sendEvent("error", { message: "Invalid command: expected an object with a string method" });
    return;
  }

  switch (cmd.method) {
    case "send": {
      if (!cmd.params || typeof cmd.params !== "object" || Array.isArray(cmd.params)) {
        sendEvent("error", { message: "Invalid send params: expected an object" });
        return;
      }
      sendEvent("message", {
        user_id: "echo-user",
        user_name: "Echo Bot (Node)",
        text: `Echo: ${cmd.params?.text || ""}`,
        channel_id: cmd.params?.channel_id || "default",
      });
      break;
    }
    case "shutdown":
      process.exit(0);
      break;
    default:
      sendEvent("error", { message: `Unsupported command: ${cmd.method}` });
  }
}

// Signal readiness
sendEvent("ready");

// Read commands from stdin
const rl = readline.createInterface({ input: process.stdin });
rl.on("line", (line) => {
  try {
    handleCommand(JSON.parse(line));
  } catch (e) {
    sendEvent("error", { message: `Invalid JSON: ${e.message}` });
  }
});
