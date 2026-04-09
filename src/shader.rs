use anyhow::{Context, Result};
use std::ffi::CString;

pub fn create_shader_program(vertex_src: &str, fragment_src: &str) -> Result<u32> {
    // Create vertex shader
    let vert_shader_source = CString::new(vertex_src)
        .context("Failed to create CString for vertex shader")?;
    
    let vert_shader = unsafe { gl::CreateShader(gl::VERTEX_SHADER) };
    
    unsafe {
        gl::ShaderSource(
            vert_shader,
            1,
            &vert_shader_source.as_ptr(),
            std::ptr::null(),
        );
        gl::CompileShader(vert_shader);
        
        // Check compilation status
        let mut success = gl::FALSE as gl::types::GLint;
        gl::GetShaderiv(vert_shader, gl::COMPILE_STATUS, &mut success);
        
        if success != gl::TRUE as gl::types::GLint {
            let mut error_log_size: gl::types::GLint = 0;
            gl::GetShaderiv(vert_shader, gl::INFO_LOG_LENGTH, &mut error_log_size);
            
            let mut error_log: Vec<u8> = Vec::with_capacity(error_log_size as usize);
            gl::GetShaderInfoLog(
                vert_shader,
                error_log_size,
                &mut error_log_size,
                error_log.as_mut_ptr() as *mut _,
            );
            
            error_log.set_len(error_log_size as usize);
            let log = String::from_utf8(error_log)
                .context("Failed to convert shader error log to UTF-8")?;
            
            return Err(anyhow::anyhow!("Vertex shader compilation failed: {}", log));
        }
    }
    
    // Create fragment shader
    let frag_shader_source = CString::new(fragment_src)
        .context("Failed to create CString for fragment shader")?;
    
    let frag_shader = unsafe { gl::CreateShader(gl::FRAGMENT_SHADER) };
    
    unsafe {
        gl::ShaderSource(
            frag_shader,
            1,
            &frag_shader_source.as_ptr(),
            std::ptr::null(),
        );
        gl::CompileShader(frag_shader);
        
        // Check compilation status
        let mut success = gl::FALSE as gl::types::GLint;
        gl::GetShaderiv(frag_shader, gl::COMPILE_STATUS, &mut success);
        
        if success != gl::TRUE as gl::types::GLint {
            let mut error_log_size: gl::types::GLint = 0;
            gl::GetShaderiv(frag_shader, gl::INFO_LOG_LENGTH, &mut error_log_size);
            
            let mut error_log: Vec<u8> = Vec::with_capacity(error_log_size as usize);
            gl::GetShaderInfoLog(
                frag_shader,
                error_log_size,
                &mut error_log_size,
                error_log.as_mut_ptr() as *mut _,
            );
            
            error_log.set_len(error_log_size as usize);
            let log = String::from_utf8(error_log)
                .context("Failed to convert shader error log to UTF-8")?;
            
            return Err(anyhow::anyhow!("Fragment shader compilation failed: {}", log));
        }
    }
    
    // Create shader program
    let shader_program = unsafe { gl::CreateProgram() };
    
    unsafe {
        gl::AttachShader(shader_program, vert_shader);
        gl::AttachShader(shader_program, frag_shader);
        gl::LinkProgram(shader_program);
        
        // Check linking status
        let mut success = gl::FALSE as gl::types::GLint;
        gl::GetProgramiv(shader_program, gl::LINK_STATUS, &mut success);
        
        if success != gl::TRUE as gl::types::GLint {
            let mut error_log_size: gl::types::GLint = 0;
            gl::GetProgramiv(shader_program, gl::INFO_LOG_LENGTH, &mut error_log_size);
            
            let mut error_log: Vec<u8> = Vec::with_capacity(error_log_size as usize);
            gl::GetProgramInfoLog(
                shader_program,
                error_log_size,
                &mut error_log_size,
                error_log.as_mut_ptr() as *mut _,
            );
            
            error_log.set_len(error_log_size as usize);
            let log = String::from_utf8(error_log)
                .context("Failed to convert program error log to UTF-8")?;
            
            return Err(anyhow::anyhow!("Shader program linking failed: {}", log));
        }
        
        // Clean up shaders
        gl::DetachShader(shader_program, vert_shader);
        gl::DetachShader(shader_program, frag_shader);
        gl::DeleteShader(vert_shader);
        gl::DeleteShader(frag_shader);
    }
    
    Ok(shader_program)
}