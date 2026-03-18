/**
 * Basic example — create an agent and chat with it.
 *
 * Usage:
 *   node basic.js
 */

const { LibreFang } = require("../index");

async function main() {
  const client = new LibreFang("http://localhost:4545");

  // Check server health
  const health = await client.health();
  console.log("Server:", health);

  // List existing agents
  const agents = await client.agents.list();
  console.log("Agents:", agents.length);

  let agent;
  let shouldDelete = false;

  // Use existing agent or create a new one with unique name
  if (agents.length > 0) {
    // Use the first available agent
    agent = agents[0];
    console.log("Using existing agent:", agent.id);
  } else {
    // Create a new agent with a unique name to avoid conflicts
    const timestamp = Date.now();
    agent = await client.agents.create({
      template: "assistant",
      name: `sdk-test-${timestamp}`
    });
    shouldDelete = true;
    console.log("Created agent:", agent.id);
  }

  // Send a message and get the full response
  console.log("\n--- Sending message ---");
  const reply = await client.agents.message(agent.id, "Say hello in 5 words.");
  console.log("Reply:", reply);

  // Clean up only if we created it
  if (shouldDelete) {
    await client.agents.delete(agent.id);
    console.log("Agent deleted.");
  }
}

main().catch(console.error);
