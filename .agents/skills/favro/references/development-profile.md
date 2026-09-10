# Development profile

Enable these conventions only for projects that use Git-based software delivery.

- Code cards use the minimal `code` template.
- Review includes focused correctness, regression, security, and relevant test/build checks.
- On approval, the reviewer may commit the approved repository change with the card reference when project policy authorizes it.
- Never push unless separately authorized, never amend unrelated history, and never commit plaintext credentials.
- The commit or review artifact is recorded through `favro set-result`; a commit is evidence, not the definition of completion.
- A failed review returns the card to In Progress with a concise role-prefixed explanation and actionable findings.

Projects migrating from the old five-lane workflow can keep `Q&A` as the configured review lane and their existing commit-on-approval policy in project-specific agent instructions.
