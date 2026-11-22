A completed delegation now forwards its result through the adapter the conversation arrived on, not the agent's home adapter, and the wake-idle path has the coverage the mid-turn one already had.
  The async-task entry carried only `chat_id`, so a task created in one bot's conversation was delivered by whichever adapter the agent counted as home — a different sender in the same group, or `chat not found` when that bot was not a member.
  The entry now carries the inbound `sender_account_id`, threaded from `agent_send` through `KernelHandle::send_to_agent_async_tracked` and `register_async_task`; the forward prefers it and falls back to the home instance for workflows, background processes and callerless sends, which have no inbound account.
  Reaching the wake-idle branch needs an idle agent, so it had no test at all: the forward there could be reverted to the channel-typed send — the regression this series started from — with the whole suite green.
  A mocked-backend test now drives that branch end to end, and a completed task from a sibling bot's conversation is asserted to arrive on that bot.
  (#8774) (@nevgenov)
