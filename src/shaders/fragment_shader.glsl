#version 330 core

// Usamos un buffer uniforme para los colores. El 'binding = 0' lo conecta con el lado de Rust.
layout(std140, binding = 0) uniform GradientColors {
    vec4 gradient_colors[8]; // Asumimos un máximo de 8 colores
    int gradient_colors_size;
};

uniform vec2 WindowSize;
out vec4 fragColor;

void main() {
    if (gradient_colors_size == 1) {
        fragColor = gradient_colors[0];
    } else {
        // Usamos 'gl_FragCoord' para determinar la posición vertical y elegir el color.
        float findex = (gl_FragCoord.y * float(gradient_colors_size - 1)) / WindowSize.y;
        int index = int(findex);
        float step = findex - float(index);
        if (index == gradient_colors_size - 1) {
            index--;
        }
        fragColor = mix(gradient_colors[index], gradient_colors[index + 1], step);
    }
}