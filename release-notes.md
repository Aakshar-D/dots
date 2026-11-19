<!-- dots:notes 0.5.4 -->
- Project memory: sols that work in the same project folder now share what they learn about it. Each fact is a small file in the project's `.sage/memory` folder, which you can read, edit or delete yourself (#127)
- Memory has kinds: core facts that are always there, what each run did, facts and decisions, and steps that worked. A sol's Memory tab shows "This sol" and "The project", grouped by kind (#127, #129)
- Sols remember by themselves: after a run that did real work, Sage saves one line of what it did and at most three facts worth keeping. Settings > Runs & server > Memory turns this off (#129)
- Memory costs less on every run: a sol gets a short list of the memories that fit its task and reads the rest only when it needs them, instead of everything each time (#127, #128)
- Recall finds memories by how well they match, and by the file they are about. The project graph names the memories about a file (#128)
