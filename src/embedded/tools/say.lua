-- sshwarma:say - Send message to room
return function(args)
    local message = args and args.message or args
    if not message then
        return {success = false, error = "message required"}
    end
    return tools.say(message)
end
