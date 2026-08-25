/** Stream an isolated agent response through the LibreFang API. */

"use strict";

const { LibreFang } = require("../index");

async function main() {
  const baseUrl = process.env.LIBREFANG_URL || "http://localhost:4545";
  const client = new LibreFang(baseUrl);
  let agentId;
  let activeError;
  let interrupted = false;

  const onSignal = (signal) => {
    interrupted = true;
    process.exitCode = signal === "SIGINT" ? 130 : 143;
    console.error(`\n[Interrupted] Received ${signal}; cleaning up...`);
  };
  process.once("SIGINT", onSignal);
  process.once("SIGTERM", onSignal);

  try {
    const agent = await client.agents.spawnAgent({
      template: "assistant",
      name: `sdk-stream-${Date.now()}`,
    });
    if (!agent || typeof agent.agent_id !== "string" || !agent.agent_id) {
      throw new Error("spawn response is missing a valid agent_id");
    }
    agentId = agent.agent_id;
    console.log("Created agent:", agentId);

    console.log("\n--- Streaming response ---");
    const events = client.agents.sendMessageStream(agentId, {
      message: "Say hello in 3 words.",
    });
    for await (const event of events) {
      if (event.error) {
        throw new Error(`stream failed: ${event.error}`);
      }
      if (event.content) {
        process.stdout.write(event.content);
      } else if (event.tool) {
        console.log(`\n[Tool call: ${event.tool}]`);
      } else if (event.done) {
        console.log("\n--- Done ---");
      }
      if (interrupted) break;
    }
  } catch (error) {
    activeError = error;
    throw error;
  } finally {
    process.removeListener("SIGINT", onSignal);
    process.removeListener("SIGTERM", onSignal);
    if (agentId) {
      try {
        await client.agents.killAgent(agentId, { confirm: true });
        console.log("Agent deleted.");
      } catch (cleanupError) {
        if (!activeError) throw cleanupError;
        console.error("[Warning] Failed to delete created agent:", cleanupError.message);
      }
    }
  }
}

main().catch((error) => {
  console.error("[Error] Streaming example failed:", error.message);
  process.exitCode = 1;
});
