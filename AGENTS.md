# Agent instructions

For agents, follow the fllowing workflow when developing:

1. perform the prompt.
2. if code was not modified skip the following steps.
3. run `just fix`
4. run `just test`. Fix all failures.
5. unless the prompt says not to, commit and push the code
   5a. use the conventional commit format for commit messages.
   5b. The commit description must explain the problem first.
   5c. The commit description must a summary of each area modified.
