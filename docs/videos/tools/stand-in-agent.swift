// A stand-in coding agent for tutorial videos. record.zsh compiles it and
// copies it as `claude` and `codex`, so tmux reports that command name and
// Rubrique shows the session as that agent.
//
//   stand-in-agent SCREEN [AFTER]
//
// Prints the text file SCREEN. With AFTER, waits for one line of input,
// clears the screen and prints AFTER. Then waits until it is stopped.
import Foundation

func show(_ path: String) {
    guard let text = try? String(contentsOfFile: path, encoding: .utf8) else {
        FileHandle.standardError.write("cannot read \(path)\n".data(using: .utf8)!)
        exit(1)
    }
    print(text, terminator: "")
    fflush(stdout)
}

let args = CommandLine.arguments
guard args.count == 2 || args.count == 3 else {
    FileHandle.standardError.write("usage: stand-in-agent SCREEN [AFTER]\n".data(using: .utf8)!)
    exit(1)
}
show(args[1])
if args.count == 3 {
    _ = readLine()
    print("\u{1B}[2J\u{1B}[H", terminator: "")
    show(args[2])
}
while true {
    sleep(3600)
}
