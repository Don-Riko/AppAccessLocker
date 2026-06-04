use std::process::{Command, Child};

pub struct AppLauncher {
    app_executable: String,
    args: Vec<String>, // Añadimos esto para guardar los argumentos extra
}

impl AppLauncher {
    pub fn new(executable: &str, args: Vec<String>) -> Self {
        Self {
            app_executable: executable.to_string(),
            args,
        }
    }

    pub fn launch(&self) -> Result<Child, std::io::Error> {
        // Le pasamos los argumentos adicionales al comando
        Command::new(&self.app_executable)
            .args(&self.args)
            .spawn()
    }
}