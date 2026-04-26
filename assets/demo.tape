# VHS demo tape for gild
# Install vhs: brew install charmbracelet/tap/vhs
# Run: vhs assets/demo.tape

Output assets/demo.gif

Set FontSize 14
Set Width 1200
Set Height 700
Set Theme "Dracula"
Set TypingSpeed 60ms
Set Padding 20

Type "gild --add-on ownership --add-on coupling --add-on hotspot --add-on authors --add-on types"
Enter
Sleep 8s

# Navigate the table
Sleep 2s
Type "i"
Sleep 1.5s

# Cycle time window
Type "t"
Sleep 1.5s
Type "t"
Sleep 1.5s
Type "t"
Sleep 1.5s
Type "t"
Sleep 1s

# Toggle graph view
Type "g"
Sleep 2.5s
Type "g"
Sleep 1s

# Open detail view
Enter
Sleep 2.5s

# Scroll detail
Type "j"
Sleep 0.5s
Type "j"
Sleep 0.5s
Type "j"
Sleep 2s

# Back to table
Type@0ms ""
Escape
Sleep 1s

# Open files view
Type "V"
Sleep 2.5s

# Quit
Type "q"
Sleep 1s
