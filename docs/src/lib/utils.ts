export const getPathPrefix = () => {
	return process.env.NODE_ENV === 'development' ? '/template-docs' : '';
};

export const withPrefix = (path: string) => `${getPathPrefix()}${path}`;
