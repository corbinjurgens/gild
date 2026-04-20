  git-pulse — Current State                                                                                                                                                                      
                                                                                                                                                                                                 
  Solid foundation: TUI contribution analyzer with identity deduplication, caching, impact scoring, sparkline graphs. Clean 8-module architecture.                                               
                                                                                                                                                                                                 
  Expansion Ideas                                                                                                                                                                                
                                                                                                                                                                                               
  High-Value Features                                                                                                                                                                            
                                                                                                                                                                                                 
  1. Team/org grouping — Group authors into teams, show team-level stats. Useful for larger repos.                                                                                               
  2. File/directory breakdown — "Who owns src/api/?" Heatmap of ownership by path. Pairs well with CODEOWNERS generation.                                                                      
  3. Commit message analysis — Categorize work (feat/fix/refactor/docs) via conventional commits or keyword heuristics. Show what kind of work each author does.                                 
  4. PR/merge-request awareness — Detect merge commits, attribute work to PR units instead of raw commits. More meaningful for squash-merge workflows.                                           
  5. Churn detection — Flag files with high add-then-delete cycles. Identifies unstable code areas, not just author volume.                                                                      
  6. Export formats — JSON, CSV, HTML report. Makes git-pulse useful in CI pipelines, dashboards, retrospectives.                                                                                
  7. Multi-repo mode — Aggregate stats across repos. Org-wide contribution picture.                                                                                                              
  8. .mailmap support — Standard git identity mapping. Many repos already have this; reading it avoids re-answering questionnaire.                                                               
                                                                                                                                                                                                 
  UX Improvements                                                                                                                                                                                
                                                                                                                                                                                                 
  9. Detail drill-down — Select author → see their top files, busiest weeks, commit frequency heatmap (GitHub-style calendar).                                                                   
  10. Diff view — Show actual commits for selected author in time window.
  11. Search/filter — Filter by author name, file path pattern, date range from CLI.                                                                                                             
  12. Config file — .git-pulse.toml at repo root for default branch, excluded paths, team definitions. Avoids repeating CLI flags.                                                               
                                                                                                                                                                                                 
  Technical Improvements                                                                                                                                                                         
                                                                                                                                                                                                 
  13. Parallel diff computation — git2 diff stats per commit is bottleneck on large repos. Rayon could parallelize.                                                                              
  14. Incremental cache — Current cache is good but could track HEAD position to skip walking already-cached history entirely.
  15. Streaming progress — Show progress bar (indicatif) during initial scan of large repos instead of just "Scanning...".                                                                       
                                                                                                                                                                                                 
  Unique/Fun Features                                                                                                                                                                            
                                                                                                                                                                                                 
  16. "Bus factor" metric — How many authors would need to leave before a file/module has no active contributors?                                                                                
  17. Activity patterns — Time-of-day/day-of-week heatmap per author. When does the team actually work?
  18. Contribution velocity trends — Is the project accelerating or decelerating? Author ramp-up curves for onboarding visibility.                                                               
                                                                                                                                                                                                 
  Biggest bang for effort: .mailmap support (#8), file ownership (#2), and export (#6). They're each moderate scope and unlock real workflows beyond "cool TUI to look at."   
