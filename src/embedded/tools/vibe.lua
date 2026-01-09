-- sshwarma:vibe - Get or set room vibe
return function(args)
    local vibe = args and args.vibe
    if vibe then
        return tools.set_vibe(vibe)
    else
        return tools.vibe()
    end
end
