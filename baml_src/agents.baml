// Agent / workflow graph primitives for dkod.
//
// Each of these BAML functions compiles to a typed Rust function in
// `baml_client/`. dk-runner composes them into a graph at runtime — the
// `WorkflowPlan` returned by `PlanWorkflow` is the edge list that drives
// the runner's state machine.

// ---- Shared types ----------------------------------------------------------

enum AgentRole {
    Planner     @description("Decomposes the intent into a workflow DAG.")
    Coder       @description("Writes / edits code inside a session overlay.")
    Reviewer    @description("Reads a changeset diff and produces review notes.")
    Judge       @description("Scores a changeset + review pair; LLM-as-judge gate.")
    Verifier    @description("Runs tests / lint / typecheck and reports results.")
}

class WorkflowStep {
    id          string                @description("Stable step id, referenced by edges.")
    role        AgentRole
    description string                @description("What this step should accomplish.")
    tools       string[]              @description("MCP tool names this step is allowed to call.")
    inputs      string[]              @description("Step ids whose output is consumed here.")
}

class WorkflowEdge {
    from string
    to   string
    // Optional predicate. Empty string means unconditional.
    when string
}

class WorkflowPlan {
    steps WorkflowStep[]
    edges WorkflowEdge[]
    entry string                      @description("Step id to execute first.")
}

class ReviewVerdict {
    approved  bool
    score     int                     @description("0..100, higher is better.")
    rationale string
    blockers  string[]                @description("Concrete issues that must be fixed before approval.")
}

// ---- Functions -------------------------------------------------------------

/// Plan an end-to-end workflow for an agent session given a free-form intent.
/// dk-runner consumes the returned plan and walks the graph, invoking each
/// step via the matching BAML function / MCP tool.
function PlanWorkflow(intent: string, repo_context: string) -> WorkflowPlan {
    client DkodDefault
    prompt #"
        You are the orchestrator for dkod, a code-intelligence engine.
        Produce a minimal workflow plan (DAG) that accomplishes the intent
        using only the agent roles available. Keep steps small and
        independently verifiable. Every non-entry step must list at least
        one input.

        Intent:
        {{ intent }}

        Repo context:
        {{ repo_context }}

        {{ ctx.output_format }}
    "#
}

/// LLM-as-judge gate. Given a changeset summary and a reviewer's notes,
/// decide whether to approve or send back for another coder iteration.
function JudgeChangeset(summary: string, review_notes: string) -> ReviewVerdict {
    client DkodJudge
    prompt #"
        You are the final judge for a dkod changeset. Approve only when the
        reviewer's notes contain no unresolved blocking issues and the
        changeset summary matches the stated intent. Score strictly.

        Changeset summary:
        {{ summary }}

        Reviewer notes:
        {{ review_notes }}

        {{ ctx.output_format }}
    "#
}

/// Deep-review pass used by dk-runner's review gate (lib/agents nodes wire
/// through here). Returns a structured verdict + concrete blockers so the
/// UI can render actionable comments.
function DeepReview(diff: string, context: string) -> ReviewVerdict {
    client DkodAnthropic
    prompt #"
        You are a senior reviewer on the dkod team. Focus on correctness,
        safety, and consistency with the surrounding code. Be concrete.

        Diff:
        ```
        {{ diff }}
        ```

        Context:
        {{ context }}

        {{ ctx.output_format }}
    "#
}

// ---- Tests -----------------------------------------------------------------

test plan_simple_refactor {
    functions [PlanWorkflow]
    args {
        intent "Split `handle_request` into request parsing + business logic."
        repo_context "crates/dk-server/src/grpc.rs ~800 LoC, tonic handler style."
    }
}

test judge_rejects_blockers {
    functions [JudgeChangeset]
    args {
        summary "Adds retry with exponential backoff to SubmitChangeset."
        review_notes "Reviewer: unbounded retry loop can block the executor for minutes. Also no test coverage. Blocking."
    }
}
