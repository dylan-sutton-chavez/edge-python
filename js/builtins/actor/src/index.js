// A declared actor compiles anywhere, the first send says it needs the CLI.
export const actor = () => ({
    send: () => { throw new Error("actor.send needs the CLI"); },
});

export default actor;
