-- sshwarma:fork - Fork current room
return function(args)
    local name = args and args.name or args
    if not name then
        return {success = false, error = "room name required"}
    end
    return tools.fork(name)
end
