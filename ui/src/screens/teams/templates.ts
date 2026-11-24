import type { Step, Team, WorkflowSpec } from "@/api/types";

import { DEFAULT_BUDGET } from "./format";

/// A workflow to start from (plain workflows spec section 3): what it does in a line, what the
/// person still has to fill in before it can run, and the workflow itself for a team.
export type WorkflowTemplate = {
  id: string;
  name: string;
  summary: string;
  /// What to do after it is made, in the order to do it.
  needs: string[];
  build: (team: Pick<Team, "members" | "projects">) => WorkflowSpec;
};

/// A lean agent step: its member, its job, the texts it hands on and the size of its model.
function agent(
  id: string,
  name: string,
  member: string,
  job: string,
  produces: string[],
  model: "small" | "standard" | "large",
): Step {
  return {
    id,
    name,
    kind: "agent",
    member,
    job,
    can_send_back_to: [],
    outcomes: [],
    delegate: null,
    options: { model, lean: true, reads: null },
    produces,
  };
}

const check = (id: string, name: string): Step => ({ id, name, kind: "check", send_back_to: null });

/// The member that does a template's steps: the team's first.
const first = (team: Pick<Team, "members">) => team.members[0]?.dot_id ?? "";

export const TEMPLATES: WorkflowTemplate[] = [
  {
    id: "post-at-a-time",
    name: "Write a post, approve it, publish it at a time",
    summary:
      "A sol writes the post from what you tell it. You read it and can change it. Sage sends it to your posting service at the date and time you gave.",
    needs: [
      "In the step Publish it, put the address your posting service gives for a new post.",
      "Keep that service's key as a secret variable named POSTING_KEY in Library.",
    ],
    build: (team) => ({
      name: "Post at a time",
      enabled: false,
      definition: {
        trigger: { kind: "manual" },
        inputs: [
          {
            name: "publish_at",
            label: "Publish at",
            kind: "date_time",
            required: true,
            default: null,
          },
        ],
        steps: [
          agent(
            "draft",
            "Write the post",
            first(team),
            "Write one post about the task, ready to publish as it is: no notes to the reader, no placeholders.",
            ["post"],
            "standard",
          ),
          check("approve", "Read and approve"),
          { id: "hold", name: "Until it is time", kind: "wait", wait: "input", input: "publish_at" },
          {
            id: "publish",
            name: "Publish it",
            kind: "action",
            action: "http",
            method: "post",
            url: "https://example.com/your-posting-service",
            headers: [{ name: "Authorization", value: "Bearer {{secret.POSTING_KEY}}" }],
            body: '{"text": "{{draft.post}}"}',
            unattended: false,
          },
        ],
        budget: DEFAULT_BUDGET,
      },
    }),
  },
  {
    id: "answer-telegram",
    name: "Answer messages on Telegram",
    summary:
      "A message to your Telegram bot starts it. A sol writes the answer, you approve it, and Sage sends it to the same chat.",
    needs: [
      "Make a bot with @BotFather in Telegram, and keep its token as a secret variable named BOT_TOKEN in Library.",
      "In the trigger, pick the chats it listens to: write to the bot from the chat, then use Find chats.",
      "To answer with nobody looking, delete the step Read and approve and turn on Runs without my check in Send the answer.",
    ],
    build: (team) => ({
      name: "Answer on Telegram",
      enabled: false,
      definition: {
        trigger: { kind: "telegram", token: "BOT_TOKEN", chats: [], project: "" },
        inputs: [
          { name: "chat_id", label: "Chat", kind: "text", required: false, default: null },
          { name: "sender", label: "Who wrote", kind: "text", required: false, default: null },
        ],
        steps: [
          agent(
            "answer",
            "Write the answer",
            first(team),
            "The task is a message from a client. Write the answer to send back: short, friendly and complete.",
            ["reply"],
            "small",
          ),
          check("approve", "Read and approve"),
          {
            id: "send",
            name: "Send the answer",
            kind: "action",
            action: "telegram",
            token: "BOT_TOKEN",
            chat: "{{input.chat_id}}",
            text: "{{answer.reply}}",
            unattended: false,
          },
        ],
        budget: DEFAULT_BUDGET,
      },
    }),
  },
  {
    id: "draft-and-mail",
    name: "Write an email, approve it, send it",
    summary:
      "A sol writes the subject and the email from what you tell it. You read both and can change them. Sage sends it through your mail service.",
    needs: [
      "In the step Send it, put the address your mail service gives for sending, and its body as that service wants it.",
      "Keep that service's key as a secret variable named MAIL_KEY in Library.",
    ],
    build: (team) => ({
      name: "Email with approval",
      enabled: false,
      definition: {
        trigger: { kind: "manual" },
        inputs: [{ name: "to", label: "Send to", kind: "text", required: true, default: null }],
        steps: [
          agent(
            "write",
            "Write the email",
            first(team),
            "Write the email the task asks for: a subject line and the body, ready to send as they are.",
            ["subject", "body"],
            "standard",
          ),
          check("approve", "Read and approve"),
          {
            id: "send",
            name: "Send it",
            kind: "action",
            action: "http",
            method: "post",
            url: "https://example.com/your-mail-service",
            headers: [{ name: "Authorization", value: "Bearer {{secret.MAIL_KEY}}" }],
            body: '{"to": "{{input.to}}", "subject": "{{write.subject}}", "text": "{{write.body}}"}',
            unattended: false,
          },
        ],
        budget: DEFAULT_BUDGET,
      },
    }),
  },
  {
    id: "weekly-report",
    name: "A report every Monday morning",
    summary:
      "Every Monday at 9:00 a sol writes the report you describe, and Sage saves it as a file. A Monday missed while Sage was closed is caught up when it opens.",
    needs: ["In the trigger, say what the report is about, and change the day and time if you like."],
    build: (team) => ({
      name: "Monday report",
      enabled: false,
      definition: {
        trigger: {
          kind: "schedule",
          schedule: {
            rule: { kind: "weekly", days: ["mon"], time: "09:00" },
            catch_up: false,
            until: null,
          },
          task: "Write this week's report.",
          project: "",
          catch_up: true,
        },
        inputs: [],
        steps: [
          agent(
            "write",
            "Write the report",
            first(team),
            "Write the report the task asks for, as a document that stands on its own.",
            ["report"],
            "standard",
          ),
          {
            id: "save",
            name: "Save it",
            kind: "action",
            action: "file",
            path: "reports/latest.md",
            content: "{{write.report}}",
            unattended: false,
          },
        ],
        budget: DEFAULT_BUDGET,
      },
    }),
  },
  {
    id: "fix-and-ship",
    name: "Change code, check it, open a pull request",
    summary:
      "A sol makes the change in a branch of its own. You look at the diff. Sage pushes the branch and opens a pull request.",
    needs: [
      "Add a project to the team that is a git repo, and pick it when you run the workflow.",
      "The member that does the work should work in worktrees (its sol's settings).",
    ],
    build: (team) => ({
      name: "Change and ship",
      enabled: false,
      definition: {
        trigger: { kind: "manual" },
        inputs: [],
        steps: [
          {
            ...agent("work", "Make the change", first(team), "Do what the task asks.", [], "large"),
            // Code work reads the sol's memories and the project's map.
            options: { model: null, lean: false, reads: null },
          },
          check("check", "Look at the diff"),
          { id: "ship", name: "Open a pull request", kind: "deploy", mode: "pr" },
        ],
        budget: DEFAULT_BUDGET,
      },
    }),
  },
];
