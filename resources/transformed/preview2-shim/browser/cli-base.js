let _env;
export function _setEnv (envObj) {
  _env = Object.entries(envObj);
}

export const environment = {
  getEnvironment () {
    if (!_env) _env = [];
    return _env;
  }
};

class ComponentExit extends Error {
  constructor(code) {
    super(`Component exited ${code === 0 ? 'successfully' : 'with error'}`);
    this.code = code;
  }
}

export const exit = {
  exit (status) {
    throw new ComponentExit(status.tag === 'err' ? 1 : 0);
  }
};

export const preopens = {
  getDirectories () {
    return [];
  }
}

export const stdin = {
  getStdin () {
    return 0;
  }
};

export const stdout = {
  getStdout () {
    return 1;
  }
};

export const stderr = {
  getStderr () {
    return 2;
  }
};
