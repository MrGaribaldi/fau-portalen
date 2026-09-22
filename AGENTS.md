# FAU workspace

Read docs/pre-0-guide.md and docs/planning-decisions.md before infrastructure work.
Use .agents/skills/favro/SKILL.md for Favro and preserve this skill reference.
The user approves all Favro commands; do not request confirmation again for Favro commands.
This approval includes all sub-agents: reading Favro cards is always authorized for the primary agent and every sub-agent.
Use .agents/skills/fau-vpn/SKILL.md to reach the private 10.0.0.0/14 network; the
WireGuard tunnel dies with the container and nothing raises it at start.
Existing collection: FAU-plattform. Pre-0 card: #3438; integration card: #3424.
Project root configuration belongs here; /opt/infra-tools is a read-only module source.
Terraform runs in /infra-runtime/infrastructure/<stage> with private state and credentials.
Never expose credentials, state, auth files or private keys in chat or Favro.
Local preparation and plan are authorized. Present actual changes before apply or reinstall.
S3 remains enabled; extensive SSD backup is deferred for the MVP test.
Single physical host failure is accepted for this phase; preserve future data portability.
Terraform step 0 generates SSH keys; do not ask for a separately generated key.
Use only this project's Compose services. Keep app runtime separate from agent tooling.
Do not rebuild/restart the agent service from inside itself; hand image changes to the host.

Read access to all files under /workspace is authorized, including sed and similar tools.
For major product or technology decisions, create a separate Favro decision card with an attached alternatives comparison: pros and cons for each option, followed by a recommendation at the bottom.
