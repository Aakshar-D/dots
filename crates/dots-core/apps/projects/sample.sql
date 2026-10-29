-- The review card's preview of a change to Projects runs on these rows.
INSERT INTO projects (id, name, position) VALUES ('p1', 'Website relaunch', 1);
INSERT INTO projects (id, name, position) VALUES ('p2', 'Home move', 2);
INSERT INTO sections (id, project_id, name, position) VALUES ('s1', 'p1', 'To do', 1);
INSERT INTO sections (id, project_id, name, position) VALUES ('s2', 'p1', 'Done', 2);
INSERT INTO sections (id, project_id, name, position) VALUES ('s3', 'p2', 'Boxes', 1);
INSERT INTO tasks (id, project_id, section_id, title, spec, assignee_kind, assignee_dot_id, due_on,
  status, position, created_at, updated_at, completed_at)
VALUES
  ('t1', 'p1', 's1', 'Write pricing page copy', 'Three plans; keep it short.', 'person', NULL,
   '2026-10-28', 'to_do', 1, '2026-10-20T09:00:00+00:00', '2026-10-20T09:00:00+00:00', NULL),
  ('t2', 'p1', 's1', 'Build signup form',
   'Email and password, inline errors, posts to `/api/signup`.', NULL, NULL, '2026-10-25',
   'in_progress', 2, '2026-10-20T09:05:00+00:00', '2026-10-21T10:00:00+00:00', NULL),
  ('t3', 'p1', 's2', 'Pick hosting', '', 'person', NULL, '2026-10-20', 'done', 1,
   '2026-10-18T09:00:00+00:00', '2026-10-20T16:00:00+00:00', '2026-10-20T16:00:00+00:00'),
  ('t4', NULL, NULL, 'Call the landlord', '', NULL, NULL, NULL, 'to_do', 1,
   '2026-10-21T08:00:00+00:00', '2026-10-21T08:00:00+00:00', NULL);
INSERT INTO subtasks (id, task_id, title, done, position) VALUES
  ('u1', 't2', 'Form markup', 1, 1),
  ('u2', 't2', 'Validation', 0, 2),
  ('u3', 't2', 'Tests', 0, 3);
INSERT INTO activity (id, task_id, at, actor_kind, actor_dot_id, actor_name, kind, field,
  before_json, after_json, text, run_id)
VALUES
  ('a1', 't2', '2026-10-20T09:05:00+00:00', 'person', NULL, 'You', 'change', 'created', NULL,
   '{"title":"Build signup form"}', NULL, NULL),
  ('a2', 't2', '2026-10-21T10:00:00+00:00', 'person', NULL, 'You', 'change', 'subtask:u1',
   '{"title":"Form markup","done":false}', '{"title":"Form markup","done":true}', NULL, NULL),
  ('a3', 't2', '2026-10-21T10:05:00+00:00', 'person', NULL, 'You', 'comment', NULL, NULL, NULL,
   'Match the pricing page style.', NULL);
