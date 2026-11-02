<!-- dots:notes 0.3.0 -->
- Accounts, part 1: your Dots ID and Profile, Organisations renamed Accounts, and an account switcher that filters Home
- Accounts, part 2: a signed member list with roles (Owner, Admin, Member, Viewer), waiting and removed states, and held runs when a role no longer allows them
- Accounts, part 3: invites, request codes and a Members tab to invite, approve, change roles and remove people, with in-app signing, publishing and drafts
- Accounts, part 4: workspaces. Split an account into workspaces with their own members' roles, filter Home by workspace, move dots between them, and narrow what each may use
- Accounts, part 5: API keys for a workspace, made in Dots, to list dots, trigger a run and read its status over the local server, plus a Host check on every route of it. If you forward webhooks through a tunnel, have it rewrite Host to 127.0.0.1:PORT (ngrok `--host-header=rewrite`, cloudflared `httpHostHeader`).
- Conversational chat: a dot answers you in the conversation and proposes new instructions for you to accept (#33)
- The conversation gets a new layout (conversational chat, part B) (#37)
- Projects, the app that comes with Dots (app library, plan C) (#35)
- An approval answered the moment it is asked now reaches the waiting dot instead of being queued (#43)
