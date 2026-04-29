Output examples/demo.gif

Set Theme "Dracula"
Set FontSize 18
Set Width 1200
Set Height 720
Set TypingSpeed 25ms
Set PlaybackSpeed 1
Set Padding 24
Set Shell zsh

Hide
Type `cd /tmp && rm -rf mdqy-demo && cp -r ~/workspace/mdqy/examples mdqy-demo && cd mdqy-demo && export PATH=$HOME/.cargo/bin:$PATH && export PS1='$ ' && clear`
Enter
Sleep 400ms
Show

Type "mdless demo.md"
Enter
Sleep 1500ms
Space
Sleep 1500ms
Space
Sleep 1500ms
Space
Sleep 1500ms
Type "q"
Sleep 300ms

Type "clear"
Enter
Sleep 200ms

Type "mdqy 'h1 | .text' demo.md"
Enter
Sleep 1200ms

Type "mdqy 'headings | .text' demo.md"
Enter
Sleep 1800ms

Type "mdqy '## Design > h3 | .text' demo.md"
Enter
Sleep 1800ms

Type "clear"
Enter
Sleep 200ms

Type "mdqy 'codeblocks | .lang' demo.md --raw | sort -u"
Enter
Sleep 1800ms

Type "mdqy 'links | .href' demo.md --raw | head -3"
Enter
Sleep 1800ms

Type "clear"
Enter
Sleep 200ms

Type `mdqy --dry-run '(.. | select(type=="link")).href |= sub("http:"; "https:")' demo.md`
Enter
Sleep 3500ms

Type "clear"
Enter
Sleep 200ms

Type `mdqy --output tty 'section("API")' demo.md`
Enter
Sleep 3000ms

Sleep 1000ms
