/**
 * Streaming example — stream agent responses token by token.
 *
 * Usage:
 *   node streaming.js
 */

const { LibreFang } = require("../index");

async function main() {
  const client = new LibreFang("http://localhost:4545");

  // List existing agents
  const agents = await client.agents.list();

  // Use existing agent or create a new one
  let agent;
  let shouldDelete = false;
  if (agents.length > 0) {
    agent = agents[0];
    console.log("Using existing agent:", agent.id);
  } else {
    agent = await client.agents.create({ template: "assistant" });
    console.log("Created agent:", agent.id);
    shouldDelete = true;
  }

  // Stream the response
  console.log("\n--- Streaming response ---");
  for await (const event of client.agents.stream(agent.id, "Say hello in 3 words.")) {
    if (event.type === "text_delta" && event.delta) {
      process.stdout.write(event.delta);
    } else if (event.type === "tool_call") {
      console.log("\n[Tool call:", event.tool, "]");
    } else if (event.type === "done") {
      console.log("\n--- Done ---");
    }
  }

  // Clean up only if we created it
  if (shouldDelete) {
    await client.agents.delete(agent.id);
  }
}

main().catch(console.error);
