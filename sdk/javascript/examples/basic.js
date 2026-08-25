/** Create an isolated agent and chat with it through the LibreFang API. */

"use strict";

const { LibreFang } = require("../index");

async function main() {
  const baseUrl = process.env.LIBREFANG_URL || "http://localhost:4545";
  const client = new LibreFang(baseUrl);
  let agentId;
  let activeError;

  try {
    console.log("Server:", await client.system.health());

    const agent = await client.agents.spawnAgent({
      template: "assistant",
      name: `sdk-test-${Date.now()}`,
    });
    if (!agent || typeof agent.agent_id !== "string" || !agent.agent_id) {
      throw new Error("spawn response is missing a valid agent_id");
    }
    agentId = agent.agent_id;
    console.log("Created agent:", agentId);

    console.log("\n--- Sending message ---");
    const reply = await client.agents.sendMessage(agentId, {
      message: "Say hello in 5 words.",
    });
    console.log("Reply:", reply);
  } catch (error) {
    activeError = error;
    throw error;
  } finally {
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
  console.error("[Error] Basic example failed:", error.message);
  process.exitCode = 1;
});
